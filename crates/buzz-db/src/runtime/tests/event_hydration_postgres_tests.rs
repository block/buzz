//! Routing behavior for the timestamp-aware production hydration seam.
use super::*;

fn key(event: &nostr::Event) -> ([u8; 32], i64) {
    (event.id.to_bytes(), event.created_at.as_secs() as i64)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn hydration_bounded_route_fence_and_replica_error_fallback() {
    let admin_url = crate::test_support::database_url();
    let (writer, _writer_db) = ScratchDb::migrated(&admin_url, "hyd_w").await;
    let (replica, _replica_db) = ScratchDb::migrated(&admin_url, "hyd_r").await;
    let keys = nostr::Keys::generate();
    let cid = Uuid::new_v4();
    let foreign = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let foreign_channel = Uuid::new_v4();
    let writer_only = signed_event_at(&keys, "writer-only", 1_768_435_200);
    let replica_only = signed_event_at(&keys, "replica-only", 1_773_532_800);
    let foreign_only = signed_event_at(&keys, "foreign-only", 1_773_532_800);
    for pool in [&writer, &replica] {
        seed_community_channel(pool, cid, channel, &keys).await;
        seed_community_channel(pool, foreign, foreign_channel, &keys).await;
    }
    insert_top_level(&writer, cid, channel, &writer_only).await;
    insert_top_level(&replica, cid, channel, &replica_only).await;
    insert_top_level(&replica, foreign, foreign_channel, &foreign_only).await;
    let community = CommunityId::from_uuid(cid);
    let hits = [key(&writer_only), key(&replica_only), key(&foreign_only)];
    let mut db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());
    // Open content fence alone cannot route an unpinned by-key read.
    let found = db
        .get_events_by_ids_at_routed("req_search_hydrate", community, &hits)
        .await
        .unwrap();
    assert_eq!(contents(&found), ["writer-only"]);
    db.set_replica_read_max_age_for_tests(Some(Duration::from_secs(30)));
    let found = db
        .get_events_by_ids_at_routed("req_search_hydrate", community, &hits)
        .await
        .unwrap();
    assert_eq!(
        contents(&found),
        ["replica-only"],
        "bounded proof must use the replica, scoped to the community"
    );
    // The heartbeat remains readable; only the proved data query fails.
    sqlx::query("DROP TABLE events CASCADE")
        .execute(&replica)
        .await
        .unwrap();
    let found = db
        .get_events_by_ids_at_routed("req_search_hydrate", community, &hits)
        .await
        .unwrap();
    assert_eq!(
        contents(&found),
        ["writer-only"],
        "non-cancel errors must retry on the writer"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn hydration_replica_cancel_is_not_retried_on_writer() {
    let fx = e_tag_fixture("hyd_cxl").await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {} SET statement_timeout = '300ms'",
        fx.replica_db.name()
    )))
    .execute(&fx.admin)
    .await
    .unwrap();
    let db = fx.routed_db().await;
    let query = EventQuery::for_community(fx.cid);
    let rows = event::query_events(&fx.replica, &query).await.unwrap();
    let hits: Vec<_> = rows.iter().map(|row| key(&row.event)).collect();
    let healthy = db
        .get_events_by_ids_at_routed("req_search_hydrate", fx.cid, &hits)
        .await
        .unwrap();
    assert!(contents(&healthy).contains(&"replica-reply"));
    let mut locker = fx.replica.begin().await.unwrap();
    sqlx::query("LOCK TABLE events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *locker)
        .await
        .unwrap();
    let error = db
        .get_events_by_ids_at_routed("req_search_hydrate", fx.cid, &hits)
        .await
        .unwrap_err();
    assert!(
        error.is_statement_cancelled(),
        "must surface replica cancellation: {error}"
    );
    locker.rollback().await.unwrap();
}
