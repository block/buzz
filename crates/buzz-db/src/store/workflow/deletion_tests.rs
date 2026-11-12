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

    let run_id = create_workflow_run(
        &db.pool,
        community,
        id,
        None,
        None,
        &crate::workflow::RunCause::Schedule,
    )
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
    sqlx::query("DELETE FROM workflows WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(id)
        .execute(&db.pool)
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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn quiescing_community_rejects_workflow_deletion_at_admission_before_replacement_lock() {
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
        .expect("sign deletion");
    crate::test_support::quiesce_community_for_tests(&db.pool, community).await;

    // Hold the workflow coordinate's replacement lock. Admission must reject
    // both entry points before they reach it, so neither queues behind it.
    let mut holder = db.pool.begin().await.expect("begin lock holder");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(crate::store::replaceable::event_replacement_lock_key(
            community,
            KIND_WORKFLOW_DEF as i32,
            &owner,
            Some(d_tag.as_bytes()),
        ))
        .execute(&mut *holder)
        .await
        .expect("hold replacement lock");
    let bounded = std::time::Duration::from_secs(5);
    let by_coordinate = tokio::time::timeout(
        bounded,
        db.delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64),
    )
    .await
    .expect("admission must reject before waiting on the replacement lock")
    .expect_err("a quiescing community must reject coordinate deletion");
    let with_request = tokio::time::timeout(
        bounded,
        db.insert_workflow_deletion(community, &deletion, &owner, &d_tag),
    )
    .await
    .expect("admission must reject before any deletion write")
    .expect_err("a quiescing community must reject a deletion request");
    holder.rollback().await.expect("release replacement lock");

    for error in [&by_coordinate, &with_request] {
        assert!(
            crate::test_support::is_admission_rejection(error),
            "expected entry admission rejection, got: {error:#}"
        );
    }
    assert_present(&db, &query, id).await;
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(deletion.id.as_bytes().as_slice())
            .fetch_one(&db.pool)
            .await
            .expect("count deletion request");
    assert_eq!(stored, 0, "a rejected deletion request must not be stored");
}

async fn count_where(db: &Db, table: &str, column: &str, community: CommunityId, id: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE community_id = $1 AND {column} = $2"
    )))
    .bind(community.as_uuid())
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("count children")
}

/// With every foreign-key delete action retired, the coordinate delete must
/// remove approvals (of the workflow or of its runs), fires and runs itself,
/// in an order the remaining constraints accept.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_deletion_removes_children_without_cascades() {
    let (db, community) = setup().await;
    crate::test_support::retire_foreign_key_delete_actions(&db.pool).await;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    let query = seed(&db, community, &keys, id, &d_tag, now).await;
    let other = Uuid::new_v4();
    seed(&db, community, &keys, other, &other.to_string(), now).await;
    let run_id = create_workflow_run(
        &db.pool,
        community,
        id,
        None,
        None,
        &crate::workflow::RunCause::Schedule,
    )
    .await
    .expect("run");
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
    // One approval hangs off the workflow; the other names a different
    // workflow but this workflow's run, so only the run_id branch reaches it.
    for (token, workflow_id) in [("workflow-approval", id), ("run-approval", other)] {
        crate::workflow::create_approval(
            &db.pool,
            crate::workflow::CreateApprovalParams {
                community_id: community,
                token,
                workflow_id,
                run_id,
                step_id: "gate",
                step_index: 0,
                approver_spec: "@anyone",
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            },
        )
        .await
        .expect("approval");
    }

    let outcome = db
        .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
        .await
        .expect("delete without cascades");
    assert!(outcome.changed);
    assert_absent(&db, &query, id).await;
    for (table, column, key) in [
        ("workflow_approvals", "workflow_id", id),
        ("workflow_approvals", "run_id", run_id),
        ("scheduled_workflow_fires", "workflow_id", id),
        ("workflow_runs", "workflow_id", id),
    ] {
        assert_eq!(
            count_where(&db, table, column, community, key).await,
            0,
            "{table} rows by {column} must be deleted explicitly"
        );
    }
    assert!(get_workflow(&db.pool, community, other).await.is_ok());
}

/// A run inserted concurrently with the delete holds the workflow's key-share
/// lock, taken by the run writer's reference check. The delete must wait for it
/// at its row lock and then remove the run; if it took its child snapshot
/// first, the run would commit after the child delete and dangle. The
/// run-to-workflow foreign key is dropped, so only the explicit lock can make
/// the delete wait.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_deletion_waits_for_and_removes_concurrent_run() {
    let (db, community) = setup().await;
    crate::test_support::retire_foreign_key_delete_actions(&db.pool).await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("workflow_runs", "workflows")])
        .await;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    seed(&db, community, &keys, id, &d_tag, now).await;

    let mut inserter = db.pool.begin().await.expect("begin run insert");
    super::super::require_workflow(&mut inserter, community, id)
        .await
        .expect("lock workflow as the run writer does");
    sqlx::query(
        "INSERT INTO workflow_runs (community_id, id, workflow_id, status, current_step, execution_trace) \
         VALUES ($1, $2, $3, 'pending', 0, '[]')",
    )
    .bind(community.as_uuid())
    .bind(Uuid::new_v4())
    .bind(id)
    .execute(&mut *inserter)
    .await
    .expect("insert concurrent run");

    let name = format!("workflow-delete-{}", Uuid::new_v4().simple());
    let deleter = Db::from_pool(crate::test_support::named_pool(&name).await);
    let deletion = tokio::spawn(async move {
        deleter
            .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
            .await
    });
    crate::test_support::wait_for_lock_wait(&db.pool, &name).await;
    inserter.commit().await.expect("commit concurrent run");

    let outcome = deletion
        .await
        .expect("join deletion")
        .expect("delete after concurrent run");
    assert!(outcome.changed);
    assert_eq!(
        count_where(&db, "workflow_runs", "workflow_id", community, id).await,
        0
    );
}

/// An approval naming another workflow but one of this workflow's runs only
/// key-share locks that run (in its reference check), so the workflow lock does
/// not exclude it. The delete must lock the runs, wait for the insert, and then
/// remove it; if the approval committed between the approval and run deletes it
/// would dangle. The approval foreign keys are dropped, so only the explicit
/// locks can make the delete wait.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_deletion_waits_for_and_removes_concurrent_cross_workflow_approval() {
    let (db, community) = setup().await;
    crate::test_support::retire_foreign_key_delete_actions(&db.pool).await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[
            ("workflow_approvals", "workflows"),
            ("workflow_approvals", "workflow_runs"),
        ],
    )
    .await;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    seed(&db, community, &keys, id, &d_tag, now).await;
    let other = Uuid::new_v4();
    seed(&db, community, &keys, other, &other.to_string(), now).await;
    let run_id = create_workflow_run(
        &db.pool,
        community,
        id,
        None,
        None,
        &crate::workflow::RunCause::Schedule,
    )
    .await
    .expect("run");

    let mut inserter = db.pool.begin().await.expect("begin approval insert");
    super::super::require_workflow(&mut inserter, community, other)
        .await
        .expect("lock workflow as the approval writer does");
    super::super::require_workflow_run(&mut inserter, community, run_id)
        .await
        .expect("lock run as the approval writer does");
    sqlx::query(
        "INSERT INTO workflow_approvals (community_id, token, workflow_id, run_id, step_id, \
         step_index, approver_spec, status, expires_at) \
         VALUES ($1, $2, $3, $4, 'gate', 0, '@anyone', 'pending', NOW() + INTERVAL '1 hour')",
    )
    .bind(community.as_uuid())
    .bind(Uuid::new_v4().as_bytes().to_vec())
    .bind(other)
    .bind(run_id)
    .execute(&mut *inserter)
    .await
    .expect("insert concurrent cross-workflow approval");

    let name = format!("workflow-delete-{}", Uuid::new_v4().simple());
    let deleter = Db::from_pool(crate::test_support::named_pool(&name).await);
    let deletion = tokio::spawn(async move {
        deleter
            .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
            .await
    });
    crate::test_support::wait_for_lock_wait(&db.pool, &name).await;
    inserter.commit().await.expect("commit concurrent approval");

    let outcome = deletion
        .await
        .expect("join deletion")
        .expect("delete after concurrent approval");
    assert!(outcome.changed);
    for (table, column, key) in [
        ("workflow_approvals", "run_id", run_id),
        ("workflow_runs", "workflow_id", id),
    ] {
        assert_eq!(
            count_where(&db, table, column, community, key).await,
            0,
            "{table}"
        );
    }
    assert!(get_workflow(&db.pool, community, other).await.is_ok());
}

/// Attaching a run to a scheduled fire locks the fire row, then key-share
/// locks the run for the fire's run foreign key, which this test keeps (the
/// attach writer is unchanged). The delete must take those two in the same
/// order (fire before run), so an attach in flight when the delete starts
/// completes and is then removed, rather than deadlocking with it.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_deletion_waits_for_concurrent_fire_attach_without_deadlock() {
    let (db, community) = setup().await;
    crate::test_support::retire_foreign_key_delete_actions(&db.pool).await;
    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let d_tag = id.to_string();
    let now = Timestamp::now().as_secs();
    seed(&db, community, &keys, id, &d_tag, now).await;
    let run_id = create_workflow_run(
        &db.pool,
        community,
        id,
        None,
        None,
        &crate::workflow::RunCause::Schedule,
    )
    .await
    .expect("run");
    let scheduled_for: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "INSERT INTO scheduled_workflow_fires (community_id, workflow_id, scheduled_for) \
         VALUES ($1, $2, date_trunc('second', NOW())) RETURNING scheduled_for",
    )
    .bind(community.as_uuid())
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("unlinked scheduled claim");

    // Hold the fire row as an attach does before its foreign-key check runs.
    let mut attacher = db.pool.begin().await.expect("begin fire attach");
    sqlx::query(
        "SELECT 1 FROM scheduled_workflow_fires \
         WHERE community_id = $1 AND workflow_id = $2 AND scheduled_for = $3 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(scheduled_for)
    .execute(&mut *attacher)
    .await
    .expect("lock fire row");

    let name = format!("workflow-delete-{}", Uuid::new_v4().simple());
    let deleter = Db::from_pool(crate::test_support::named_pool(&name).await);
    let deletion = tokio::spawn(async move {
        deleter
            .delete_workflow_by_coordinate(community, &owner, &d_tag, now as i64)
            .await
    });
    crate::test_support::wait_for_lock_wait(&db.pool, &name).await;
    let attached = sqlx::query(
        "UPDATE scheduled_workflow_fires SET workflow_run_id = $4 \
         WHERE community_id = $1 AND workflow_id = $2 AND scheduled_for = $3",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(scheduled_for)
    .bind(run_id)
    .execute(&mut *attacher)
    .await
    .expect("attach run to fire without deadlock");
    assert_eq!(attached.rows_affected(), 1);
    attacher.commit().await.expect("commit fire attach");

    let outcome = deletion
        .await
        .expect("join deletion")
        .expect("delete after concurrent fire attach");
    assert!(outcome.changed);
    for (table, column, key) in [
        ("scheduled_workflow_fires", "workflow_id", id),
        ("workflow_runs", "workflow_id", id),
    ] {
        assert_eq!(
            count_where(&db, table, column, community, key).await,
            0,
            "{table}"
        );
    }
}

/// Take the locks a workflow delete takes first: the workflow row and its
/// runs, `FOR UPDATE`. The caller finishes the delete with
/// [`delete_workflow_with_children`] and commits.
async fn begin_delete_holding_locks(db: &Db, community: CommunityId, id: Uuid) -> AdmittedTx {
    let mut tx = db
        .begin_event_write_transaction(community)
        .await
        .expect("begin delete");
    sqlx::query("SELECT id FROM workflows WHERE community_id = $1 AND id = $2 FOR UPDATE")
        .bind(community.as_uuid())
        .bind(id)
        .execute(tx.conn())
        .await
        .expect("lock workflow");
    sqlx::query(
        "SELECT id FROM workflow_runs WHERE community_id = $1 AND workflow_id = $2 \
         ORDER BY id FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(id)
    .execute(tx.conn())
    .await
    .expect("lock runs");
    tx
}

/// Run `writer` on its own connection while `holder` has the workflow locked,
/// wait until it is blocked on a lock, then finish the delete and commit.
/// Returns the writer's result, which ran strictly after the delete.
async fn write_while_delete_holds_locks<T: Send + 'static>(
    db: &Db,
    mut holder: AdmittedTx,
    workflow_id: Uuid,
    writer: impl FnOnce(Db) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>,
) -> T {
    let name = format!("workflow-writer-{}", Uuid::new_v4().simple());
    let writer_db = Db::from_pool(crate::test_support::named_pool(&name).await);
    let write = tokio::spawn(writer(writer_db));
    crate::test_support::wait_for_lock_wait(&db.pool, &name).await;
    delete_workflow_with_children(&mut holder, workflow_id)
        .await
        .expect("delete workflow");
    holder.commit().await.expect("commit delete");
    write.await.expect("join writer")
}

/// With the fire-to-workflow foreign key dropped, a fire claim racing a
/// workflow delete must wait on the workflow row (its `FOR KEY SHARE`) and then
/// find no workflow. Without that lock the claim reads its snapshot while the
/// delete is uncommitted, inserts after the delete removed the fires, and leaves
/// an orphan fire nothing cleans up.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn fire_claim_waits_for_workflow_delete_and_leaves_no_orphan_fire() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[("scheduled_workflow_fires", "workflows")],
    )
    .await;
    let keys = Keys::generate();
    let id = Uuid::new_v4();
    seed(
        &db,
        community,
        &keys,
        id,
        &id.to_string(),
        Timestamp::now().as_secs(),
    )
    .await;

    let holder = begin_delete_holding_locks(&db, community, id).await;
    let claim = write_while_delete_holds_locks(&db, holder, id, move |writer| {
        Box::pin(async move {
            writer
                .claim_scheduled_workflow_fire(community, id, chrono::Utc::now())
                .await
        })
    })
    .await;

    assert!(
        matches!(claim, Ok(None)),
        "a claim after the delete must lose, got {claim:?}"
    );
    assert_eq!(
        count_where(
            &db,
            "scheduled_workflow_fires",
            "workflow_id",
            community,
            id
        )
        .await,
        0,
        "no orphan fire may survive the delete"
    );
}

/// A run writer racing a workflow delete waits on the workflow row and then
/// fails, instead of committing a run for a workflow that is gone.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn run_create_waits_for_workflow_delete_and_leaves_no_orphan_run() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("workflow_runs", "workflows")])
        .await;
    let keys = Keys::generate();
    let id = Uuid::new_v4();
    seed(
        &db,
        community,
        &keys,
        id,
        &id.to_string(),
        Timestamp::now().as_secs(),
    )
    .await;

    let holder = begin_delete_holding_locks(&db, community, id).await;
    let created = write_while_delete_holds_locks(&db, holder, id, move |writer| {
        Box::pin(async move { writer.create_workflow_run(community, id, None, None).await })
    })
    .await;

    assert!(
        matches!(created, Err(DbError::NotFound(_))),
        "a run created after the delete must be rejected, got {created:?}"
    );
    assert_eq!(
        count_where(&db, "workflow_runs", "workflow_id", community, id).await,
        0,
        "no orphan run may survive the delete"
    );
}

/// An approval racing the delete of the workflow it names, or of the run it
/// names under another workflow, waits on that parent's lock and then fails.
async fn approval_waits_for_delete_and_leaves_no_orphan_approval(names_deleted_workflow: bool) {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[
            ("workflow_runs", "workflows"),
            ("workflow_approvals", "workflows"),
            ("workflow_approvals", "workflow_runs"),
        ],
    )
    .await;
    let keys = Keys::generate();
    let now = Timestamp::now().as_secs();
    let id = Uuid::new_v4();
    seed(&db, community, &keys, id, &id.to_string(), now).await;
    let other = Uuid::new_v4();
    seed(&db, community, &keys, other, &other.to_string(), now).await;
    let run_id = create_workflow_run(&db.pool, community, id, None, None)
        .await
        .expect("run");
    // Either the approval names the workflow being deleted, or it names the
    // surviving workflow and only its run belongs to the deleted one.
    let approval_workflow = if names_deleted_workflow { id } else { other };

    let holder = begin_delete_holding_locks(&db, community, id).await;
    let approved = write_while_delete_holds_locks(&db, holder, id, move |writer| {
        Box::pin(async move {
            writer
                .create_approval(crate::workflow::CreateApprovalParams {
                    community_id: community,
                    token: "racing-approval",
                    workflow_id: approval_workflow,
                    run_id,
                    step_id: "gate",
                    step_index: 0,
                    approver_spec: "@anyone",
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                })
                .await
        })
    })
    .await;

    assert!(
        matches!(approved, Err(DbError::NotFound(_))),
        "an approval after the delete must be rejected, got {approved:?}"
    );
    assert_eq!(
        count_where(&db, "workflow_approvals", "run_id", community, run_id).await,
        0,
        "no orphan approval may survive the delete"
    );
    assert!(get_workflow(&db.pool, community, other).await.is_ok());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn approval_create_waits_for_workflow_delete_and_leaves_no_orphan_approval() {
    approval_waits_for_delete_and_leaves_no_orphan_approval(true).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cross_workflow_approval_create_waits_for_run_delete_and_leaves_no_orphan_approval() {
    approval_waits_for_delete_and_leaves_no_orphan_approval(false).await;
}
