use super::scrub_postgres_tests::{options, setup, PauseAfterPlan};
use super::*;
use crate::user_deletion::ExecutionMode;
use nostr::{EventBuilder, Kind};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn event_batch_deletion_preserves_large_unrelated_delivery_history() {
    let (db, c, _, alice, _) = setup().await;
    for n in 0..100 {
        let event = EventBuilder::new(Kind::Custom(1063), format!("personal metadata {n}"))
            .sign_with_keys(&alice)
            .unwrap();
        db.insert_event(c, &event, None).await.unwrap();
    }
    // Event IDs have no delivery-log index. A per-event dependency probe scans
    // this unrelated history 100 times and exceeds the executor's 3s timeout.
    sqlx::query(
        "INSERT INTO delivery_log(community_id,subscription_id,event_id,error_message)
         SELECT $1,'unrelated',decode(md5(g::text)||md5(g::text),'hex'),repeat('x',128)
         FROM generate_series(1,500000) g",
    )
    .bind(c.as_uuid())
    .execute(db.pool())
    .await
    .unwrap();
    // Many tiny tenants make generic-plan estimates underestimate this tenant.
    // A plain IN semi join can compare every log to every event after ANALYZE.
    sqlx::query(
        "WITH tenants AS (
             INSERT INTO communities(id,host)
             SELECT gen_random_uuid(),'delivery-'||g||'.example'
             FROM generate_series(1,9999) g RETURNING id
         ) INSERT INTO delivery_log(community_id,event_id)
         SELECT id,decode(repeat('ff',32),'hex') FROM tenants",
    )
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("ANALYZE delivery_log")
        .execute(db.pool())
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(
            (*db.pool().connect_options())
                .clone()
                .options([("plan_cache_mode", "force_generic_plan")]),
        )
        .await
        .unwrap();
    let db = crate::Db::from_pool(pool);
    let mut opts = options(c, &alice);
    opts.batch_size = 100;
    for (mode, expected) in [
        (ExecutionMode::ReadOnly, 100),
        (ExecutionMode::Delete, 100),
        (ExecutionMode::Delete, 0),
    ] {
        let result = db
            .user_deletion_store()
            .run_message_batches(&opts, mode, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(result.items, expected);
    }
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(c.as_uuid())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(events, 0);
    let logs: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_log WHERE community_id=$1")
        .bind(c.as_uuid())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(logs, 500_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn historical_file_publications_and_late_delivery_copies_follow_source_scope() {
    let (db, c, _, alice, bob) = setup().await;
    let other = db
        .ensure_configured_community(&format!("copies-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let source = EventBuilder::new(Kind::Custom(1063), "personal file description")
        .sign_with_keys(&alice)
        .unwrap();
    let survivor = EventBuilder::new(Kind::Custom(1063), "another publisher's description")
        .sign_with_keys(&bob)
        .unwrap();
    for community in [c, other] {
        db.ensure_user(community, bob.public_key().as_bytes())
            .await
            .unwrap();
        db.insert_event(community, &source, None).await.unwrap();
        db.insert_event(community, &survivor, None).await.unwrap();
        sqlx::query(
            "INSERT INTO subscriptions(community_id,id,owner_pubkey) VALUES($1,'survivor',$2)",
        )
        .bind(community.as_uuid())
        .bind(bob.public_key().as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    }
    let (planned_tx, planned_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let store = db.user_deletion_store();
    let opts = options(c, &alice);
    let task = tokio::spawn(async move {
        let mut writer = PauseAfterPlan {
            record: "delete_start",
            line: Vec::new(),
            planned: Some(planned_tx),
            resume: resume_rx,
        };
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
            .await
    });
    planned_rx.await.unwrap();
    for community in [c, other] {
        for event in [&source, &survivor] {
            sqlx::query("INSERT INTO delivery_log(community_id,subscription_id,event_id,error_message) VALUES($1,'survivor',$2,'copied payload')")
                .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice())
                .execute(db.pool()).await.unwrap();
        }
    }
    resume_tx.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert!(
        error.to_string().contains("new dependent records"),
        "{error}"
    );
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(c.as_uuid())
            .bind(source.id.as_bytes().as_slice())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(
        exists,
        "source must survive until all dependent logs can be planned"
    );
    for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
        let result = db
            .user_deletion_store()
            .run_message_batches(&options(c, &alice), mode, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(result.items, 2);
    }
    for (community, expected) in [(c, 1_i64), (other, 2)] {
        for table in ["events", "delivery_log"] {
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table} WHERE community_id=$1"
            )))
            .bind(community.as_uuid())
            .fetch_one(db.pool())
            .await
            .unwrap();
            assert_eq!(count, expected, "{table}");
        }
        let subscriptions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM subscriptions WHERE community_id=$1")
                .bind(community.as_uuid())
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(subscriptions, 1);
    }
    let preserved: String =
        sqlx::query_scalar("SELECT content FROM events WHERE community_id=$1 AND id=$2")
            .bind(c.as_uuid())
            .bind(survivor.id.as_bytes().as_slice())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(preserved, survivor.content);
    assert_eq!(
        db.user_deletion_store()
            .run_message_batches(
                &options(c, &alice),
                ExecutionMode::Delete,
                None,
                &mut Vec::new()
            )
            .await
            .unwrap()
            .items,
        0
    );
}
