use super::*;
use crate::event::EventQuery;
use crate::workflow::{create_workflow_run, get_workflow, upsert_workflow};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

async fn setup() -> (Db, CommunityId) {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect test database");
    let db = Db::from_pool(pool);
    let community = db
        .ensure_configured_community(&format!("workflow-delete-{}.example", Uuid::new_v4()))
        .await
        .expect("create community")
        .id;
    (db, community)
}

async fn seed(
    db: &Db,
    community: CommunityId,
    keys: &Keys,
    workflow_id: Uuid,
    d_tag: &str,
    created_at: u64,
) -> EventQuery {
    let owner = keys.public_key().to_bytes();
    crate::user::ensure_user(&db.pool, community, &owner)
        .await
        .expect("owner");
    upsert_workflow(
        &db.pool,
        community,
        workflow_id,
        None,
        &owner,
        d_tag,
        r#"{"trigger":{"on":"manual"},"steps":[]}"#,
        &[0; 32],
    )
    .await
    .expect("seed executable workflow");
    let event = EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEF as u16),
        "name: deletion-test",
    )
    .tags([Tag::parse(["d", d_tag]).expect("d tag")])
    .custom_created_at(Timestamp::from(created_at))
    .sign_with_keys(keys)
    .expect("sign definition");
    db.replace_parameterized_event(community, &event, d_tag, None)
        .await
        .expect("seed definition");
    EventQuery {
        kinds: Some(vec![KIND_WORKFLOW_DEF as i32]),
        pubkey: Some(owner.to_vec()),
        d_tag: Some(d_tag.to_string()),
        ..EventQuery::for_community(community)
    }
}

async fn assert_present(db: &Db, query: &EventQuery, id: Uuid) {
    assert_eq!(
        db.query_events(query)
            .await
            .expect("fresh definition query")
            .len(),
        1
    );
    assert!(get_workflow(&db.pool, query.community_id, id).await.is_ok());
}

async fn assert_absent(db: &Db, query: &EventQuery, id: Uuid) {
    assert!(db
        .query_events(query)
        .await
        .expect("fresh definition query")
        .is_empty());
    assert!(matches!(
        get_workflow(&db.pool, query.community_id, id).await,
        Err(DbError::NotFound(_))
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_deletion_removes_both_projections_and_is_scoped_and_retryable() {
    let (db, community) = setup().await;
    let other = db
        .ensure_configured_community(&format!("other-{}.example", Uuid::new_v4()))
        .await
        .expect("other community")
        .id;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    let query = seed(&db, community, &keys, id, &d_tag, now).await;
    let other_query = seed(&db, other, &keys, id, &d_tag, now).await;
    assert_present(&db, &query, id).await;

    let unchanged = db
        .delete_workflow_by_coordinate(
            community,
            &Keys::generate().public_key().to_bytes(),
            &d_tag,
            now as i64,
        )
        .await
        .expect("wrong owner is a no-op");
    assert!(!unchanged.changed);
    assert_present(&db, &query, id).await;

    let run_id = create_workflow_run(&db.pool, community, id, None, None)
        .await
        .expect("run");
    // A fired schedule links both the workflow and a cascading run. Deletion
    // must also remove the claim despite its NO ACTION run foreign key.
    sqlx::query(
        "INSERT INTO scheduled_workflow_fires \
        (community_id, workflow_id, scheduled_for, workflow_run_id) VALUES ($1, $2, NOW(), $3)",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(run_id)
    .execute(&db.pool)
    .await
    .expect("scheduled claim linked to run");
    let (first, second) = tokio::join!(
        db.delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64),
        db.delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64),
    );
    let first = first.expect("first deletion");
    let second = second.expect("concurrent deletion");
    assert_ne!(
        first.changed, second.changed,
        "only one deletion changes state"
    );
    assert_eq!(
        first.channel_id, None,
        "channel-less mutation still reports change"
    );
    assert_eq!(second.channel_id, None);
    assert_absent(&db, &query, id).await;
    assert_present(&db, &other_query, id).await;
    assert!(matches!(
        db.get_workflow_run(community, run_id).await,
        Err(DbError::NotFound(_))
    ));
    let unchanged = db
        .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
        .await
        .expect("repeat deletion");
    assert!(!unchanged.changed);
    assert_absent(&db, &query, id).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_deletion_preserves_newer_definition_and_cleans_legacy_orphan() {
    let (db, community) = setup().await;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    let query = seed(&db, community, &keys, id, &d_tag, now).await;
    let unchanged = db
        .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64 - 1)
        .await
        .expect("stale deletion");
    assert!(!unchanged.changed);
    assert_present(&db, &query, id).await;

    // Old relay releases deleted the executable row, but not the definition.
    db.delete_workflow_for_owner(community, id, &owner)
        .await
        .expect("legacy deletion");
    assert_eq!(
        db.query_events(&query)
            .await
            .expect("orphan definition")
            .len(),
        1
    );
    let repaired = db
        .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
        .await
        .expect("repair deletion");
    assert!(repaired.changed, "orphan-only cleanup must dispatch");
    assert_eq!(repaired.channel_id, None);
    assert_absent(&db, &query, id).await;

    let legacy_id = Uuid::new_v4();
    let legacy = seed(&db, community, &keys, legacy_id, "legacy-name", now).await;
    db.soft_delete_by_coordinate(
        community,
        KIND_WORKFLOW_DEF as i32,
        &owner,
        "legacy-name",
        now as i64,
    )
    .await
    .expect("remove definition to leave only executable projection");
    let repaired = db
        .delete_workflow_by_coordinate(community, &owner, "legacy-name", now as i64)
        .await
        .expect("name coordinate deletion");
    assert!(repaired.changed, "projection-only cleanup must dispatch");
    assert_absent(&db, &legacy, legacy_id).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_deletion_rolls_back_both_representations_on_failure() {
    let (db, community) = setup().await;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    let query = seed(&db, community, &keys, id, &d_tag, now).await;
    // A failing event update simulates a failure after projection removal.
    // The CI runner isolates this test's database from other tests.
    sqlx::raw_sql(
        "CREATE FUNCTION reject_workflow_definition_delete() RETURNS trigger \
        LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected deletion failure'; END $$; \
        CREATE TRIGGER reject_workflow_definition_delete BEFORE UPDATE ON events \
        FOR EACH ROW WHEN (OLD.kind = 30620 AND NEW.deleted_at IS NOT NULL) \
        EXECUTE FUNCTION reject_workflow_definition_delete();",
    )
    .execute(&db.pool)
    .await
    .expect("install failure trigger");
    let result = db
        .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
        .await;
    sqlx::raw_sql(
        "DROP TRIGGER reject_workflow_definition_delete ON events; \
        DROP FUNCTION reject_workflow_definition_delete();",
    )
    .execute(&db.pool)
    .await
    .expect("remove failure trigger");
    assert!(result.is_err());
    assert_present(&db, &query, id).await;
    db.delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
        .await
        .expect("retry after failure");
    assert_absent(&db, &query, id).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_stored_deletion_repair_has_one_committed_dispatch_owner() {
    let (db, community) = setup().await;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    let query = seed(&db, community, &keys, id, &d_tag, now).await;
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags([Tag::parse([
            "a",
            &format!("30620:{}:{d_tag}", keys.public_key().to_hex()),
        ])
        .expect("coordinate")])
        .custom_created_at(Timestamp::from(now))
        .sign_with_keys(&keys)
        .expect("sign legacy deletion");
    // Older relay versions could persist the request without applying deletion.
    db.insert_event_with_thread_metadata(community, &deletion, None, None)
        .await
        .expect("legacy request");
    let (first, second) = tokio::join!(
        db.insert_workflow_deletion(community, &deletion, &owner, &d_tag),
        db.insert_workflow_deletion(community, &deletion, &owner, &d_tag),
    );
    let (_, first_dispatch, _) = first.expect("first repair");
    let (_, second_dispatch, _) = second.expect("concurrent repair");
    assert_ne!(
        first_dispatch, second_dispatch,
        "only one committed repair dispatches"
    );
    assert_absent(&db, &query, id).await;
    assert!(
        !db.insert_workflow_deletion(community, &deletion, &owner, &d_tag)
            .await
            .expect("completed duplicate")
            .1
    );
}
