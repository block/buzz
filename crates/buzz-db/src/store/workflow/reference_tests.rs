//! A writer into a workflow table rejects a missing parent in application
//! code. Each test drops the foreign keys it covers from the per-test database
//! first, so a pass proves the in-transaction check rather than the constraint.

use super::*;
use crate::user::ensure_user;

async fn setup() -> (Db, CommunityId) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect test database");
    let db = Db::from_pool(pool);
    let community = new_community(&db).await;
    (db, community)
}

async fn new_community(db: &Db) -> CommunityId {
    db.ensure_configured_community(&format!("workflow-ref-{}.example", Uuid::new_v4()))
        .await
        .expect("create community")
        .id
}

async fn new_channel(db: &Db, community: CommunityId, owner: &[u8]) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, created_by) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(community.as_uuid())
    .bind(format!("ch-{}", id.simple()))
    .bind(owner)
    .execute(&db.pool)
    .await
    .expect("insert channel");
    id
}

/// A workflow with an existing owner and no channel.
async fn new_workflow(db: &Db, community: CommunityId, owner: &[u8]) -> Uuid {
    ensure_user(&db.pool, community, owner)
        .await
        .expect("ensure owner");
    create_workflow(
        &db.pool,
        community,
        None,
        owner,
        "reference-test",
        r#"{"trigger":{"on":"manual"},"steps":[]}"#,
        &[0; 32],
    )
    .await
    .expect("create workflow")
}

async fn count(db: &Db, table: &str, community: CommunityId) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE community_id = $1"
    )))
    .bind(community.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("count rows")
}

fn assert_not_found<T: std::fmt::Debug>(result: Result<T>, what: &str) {
    match result {
        Err(DbError::NotFound(message)) => {
            assert!(message.contains(what), "unexpected message: {message}")
        }
        other => panic!("expected NotFound({what}), got {other:?}"),
    }
}

const DEFINITION: &str = r#"{"trigger":{"on":"manual"},"steps":[]}"#;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_workflow_rejects_missing_owner_or_channel_without_foreign_keys() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[("workflows", "users"), ("workflows", "channels")],
    )
    .await;
    let owner = [0x11; 32];
    let create = |channel: Option<Uuid>, owner: [u8; 32]| {
        let pool = db.pool.clone();
        async move {
            create_workflow(
                &pool, community, channel, &owner, "wf", DEFINITION, &[0; 32],
            )
            .await
        }
    };

    // Unknown owner.
    assert_not_found(create(None, owner).await, "owner");

    // The owner exists only in another community.
    let other = new_community(&db).await;
    ensure_user(&db.pool, other, &owner)
        .await
        .expect("other user");
    assert_not_found(create(None, owner).await, "owner");

    // Known owner, unknown channel; and a channel that exists only elsewhere.
    ensure_user(&db.pool, community, &owner)
        .await
        .expect("owner");
    assert_not_found(create(Some(Uuid::new_v4()), owner).await, "channel");
    let elsewhere = new_channel(&db, other, &owner).await;
    assert_not_found(create(Some(elsewhere), owner).await, "channel");
    assert_eq!(count(&db, "workflows", community).await, 0);

    // Control: valid parents are accepted.
    let channel = new_channel(&db, community, &owner).await;
    create(Some(channel), owner).await.expect("valid parents");
    assert_eq!(count(&db, "workflows", community).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn upsert_workflow_rejects_missing_owner_or_channel_without_foreign_keys() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[("workflows", "users"), ("workflows", "channels")],
    )
    .await;
    let owner = [0x12; 32];
    let upsert = |channel: Option<Uuid>, owner: [u8; 32]| {
        let pool = db.pool.clone();
        async move {
            upsert_workflow(
                &pool,
                community,
                Uuid::new_v4(),
                channel,
                &owner,
                "wf",
                DEFINITION,
                &[0; 32],
            )
            .await
        }
    };

    assert_not_found(upsert(None, owner).await, "owner");
    let other = new_community(&db).await;
    ensure_user(&db.pool, other, &owner)
        .await
        .expect("other user");
    assert_not_found(upsert(None, owner).await, "owner");

    ensure_user(&db.pool, community, &owner)
        .await
        .expect("owner");
    assert_not_found(upsert(Some(Uuid::new_v4()), owner).await, "channel");
    let elsewhere = new_channel(&db, other, &owner).await;
    assert_not_found(upsert(Some(elsewhere), owner).await, "channel");
    assert_eq!(count(&db, "workflows", community).await, 0);

    // Control: valid parents are accepted, and a retry of the same row is idempotent.
    let channel = new_channel(&db, community, &owner).await;
    let id = Uuid::new_v4();
    for _ in 0..2 {
        upsert_workflow(
            &db.pool,
            community,
            id,
            Some(channel),
            &owner,
            "wf",
            DEFINITION,
            &[0; 32],
        )
        .await
        .expect("valid parents");
    }
    assert_eq!(count(&db, "workflows", community).await, 1);

    // An existing ID: a different valid owner or channel is the overwrite
    // boundary (`AccessDenied`); a missing owner or channel fails the reference
    // check first (`NotFound`). Neither attempt touches the stored row.
    let other_owner = [0x22; 32];
    ensure_user(&db.pool, community, &other_owner)
        .await
        .expect("other owner");
    let other_channel = new_channel(&db, community, &owner).await;
    let overwrite = |channel: Option<Uuid>, owner: [u8; 32]| {
        let pool = db.pool.clone();
        async move {
            upsert_workflow(
                &pool,
                community,
                id,
                channel,
                &owner,
                "hijack",
                r#"{"trigger":{"on":"manual"},"steps":[{"id":"x"}]}"#,
                &[1; 32],
            )
            .await
        }
    };
    for (label, result) in [
        ("other owner", overwrite(Some(channel), other_owner).await),
        ("other channel", overwrite(Some(other_channel), owner).await),
    ] {
        assert!(
            matches!(result, Err(DbError::AccessDenied(_))),
            "{label}: expected AccessDenied, got {result:?}"
        );
    }
    assert_not_found(overwrite(Some(channel), [0x23; 32]).await, "owner");
    assert_not_found(overwrite(Some(Uuid::new_v4()), owner).await, "channel");

    let stored = get_workflow(&db.pool, community, id)
        .await
        .expect("stored workflow");
    assert_eq!(stored.owner_pubkey, owner.to_vec());
    assert_eq!(stored.channel_id, Some(channel));
    assert_eq!(stored.name, "wf");
    assert_eq!(stored.definition_hash, vec![0; 32]);
    assert_eq!(count(&db, "workflows", community).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_workflow_run_rejects_missing_workflow_without_foreign_key() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("workflow_runs", "workflows")])
        .await;
    let owner = [0x13; 32];

    assert_not_found(
        create_workflow_run(
            &db.pool,
            community,
            Uuid::new_v4(),
            None,
            None,
            &RunCause::Schedule,
        )
        .await,
        "workflow",
    );
    // The workflow exists only in another community.
    let other = new_community(&db).await;
    let elsewhere = new_workflow(&db, other, &owner).await;
    assert_not_found(
        create_workflow_run(
            &db.pool,
            community,
            elsewhere,
            None,
            None,
            &RunCause::Schedule,
        )
        .await,
        "workflow",
    );
    assert_eq!(count(&db, "workflow_runs", community).await, 0);
    assert_eq!(count(&db, "workflow_runs", other).await, 0);

    let workflow = new_workflow(&db, community, &owner).await;
    create_workflow_run(
        &db.pool,
        community,
        workflow,
        None,
        None,
        &RunCause::Schedule,
    )
    .await
    .expect("existing workflow");
    assert_eq!(count(&db, "workflow_runs", community).await, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_approval_rejects_missing_workflow_or_run_without_foreign_keys() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[
            ("workflow_approvals", "workflows"),
            ("workflow_approvals", "workflow_runs"),
        ],
    )
    .await;
    let owner = [0x14; 32];
    let workflow = new_workflow(&db, community, &owner).await;
    let run = create_workflow_run(
        &db.pool,
        community,
        workflow,
        None,
        None,
        &RunCause::Schedule,
    )
    .await
    .expect("run");
    let approve = |workflow_id: Uuid, run_id: Uuid, token: &'static str| {
        let pool = db.pool.clone();
        async move {
            create_approval(
                &pool,
                CreateApprovalParams {
                    community_id: community,
                    token,
                    workflow_id,
                    run_id,
                    step_id: "gate",
                    step_index: 0,
                    approver_spec: "@anyone",
                    expires_at: Utc::now() + chrono::Duration::hours(1),
                },
            )
            .await
        }
    };

    assert_not_found(
        approve(Uuid::new_v4(), run, "no-workflow").await,
        "workflow",
    );
    assert_not_found(
        approve(workflow, Uuid::new_v4(), "no-run").await,
        "workflow_run",
    );

    // Both exist, but only in another community.
    let other = new_community(&db).await;
    let other_workflow = new_workflow(&db, other, &owner).await;
    let other_run = create_workflow_run(
        &db.pool,
        other,
        other_workflow,
        None,
        None,
        &RunCause::Schedule,
    )
    .await
    .expect("other run");
    assert_not_found(
        approve(other_workflow, run, "workflow-elsewhere").await,
        "workflow",
    );
    assert_not_found(
        approve(workflow, other_run, "run-elsewhere").await,
        "workflow_run",
    );
    assert_eq!(count(&db, "workflow_approvals", community).await, 0);
    assert_eq!(count(&db, "workflow_approvals", other).await, 0);

    approve(workflow, run, "valid")
        .await
        .expect("valid parents");
    assert_eq!(count(&db, "workflow_approvals", community).await, 1);
}

/// The owner and channel checks hold `FOR KEY SHARE` on the parent rows, so a
/// concurrent delete or key update of a parent waits for the writer. The
/// foreign keys are dropped, so only the explicit lock can block the writer
/// behind a parent held `FOR UPDATE`.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_writers_key_share_lock_owner_and_channel_without_foreign_keys() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[("workflows", "users"), ("workflows", "channels")],
    )
    .await;
    let owner = [0x15; 32];
    ensure_user(&db.pool, community, &owner)
        .await
        .expect("owner");
    let channel = new_channel(&db, community, &owner).await;

    for writer_is_upsert in [false, true] {
        for lock_channel in [false, true] {
            let mut holder = db.pool.begin().await.expect("begin parent lock holder");
            if lock_channel {
                sqlx::query(
                    "SELECT 1 FROM channels WHERE community_id = $1 AND id = $2 FOR UPDATE",
                )
                .bind(community.as_uuid())
                .bind(channel)
                .execute(&mut *holder)
                .await
                .expect("lock channel");
            } else {
                sqlx::query(
                    "SELECT 1 FROM users WHERE community_id = $1 AND pubkey = $2 FOR UPDATE",
                )
                .bind(community.as_uuid())
                .bind(owner.as_slice())
                .execute(&mut *holder)
                .await
                .expect("lock owner");
            }

            let name = format!("workflow-writer-{}", Uuid::new_v4().simple());
            let pool = crate::test_support::named_pool(&name).await;
            let write = tokio::spawn(async move {
                if writer_is_upsert {
                    upsert_workflow(
                        &pool,
                        community,
                        Uuid::new_v4(),
                        Some(channel),
                        &owner,
                        "wf",
                        DEFINITION,
                        &[0; 32],
                    )
                    .await
                } else {
                    create_workflow(
                        &pool,
                        community,
                        Some(channel),
                        &owner,
                        "wf",
                        DEFINITION,
                        &[0; 32],
                    )
                    .await
                    .map(|_| ())
                }
            });
            crate::test_support::wait_for_lock_wait(&db.pool, &name).await;
            holder.commit().await.expect("release parent lock");
            write
                .await
                .expect("join writer")
                .expect("writer proceeds once the parent lock is released");
        }
    }
    assert_eq!(count(&db, "workflows", community).await, 4);
}
