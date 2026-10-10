//! Startup repair must use the complete catalog, not an interactive page.
use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn startup_repairs_past_catalog_cap_including_archived_channels() {
    let f = Fixture::new().await;
    // Fixture-only SQL: all 1,001 live channels lack discovery. The last live
    // channel is archived; the next is a tombstone and must remain absent.
    sqlx::query(
        "INSERT INTO channels (community_id, id, name, created_by, labels, archived_at, deleted_at) \
         SELECT $1, lpad(to_hex(n), 32, '0')::uuid, 'repair', $2, ARRAY['retained'], \
         CASE WHEN n = 1001 THEN now() END, CASE WHEN n = 1002 THEN now() END \
         FROM generate_series(1, 1002) AS n",
    )
    .bind(f.community.as_uuid())
    .bind(f.owner.public_key().as_bytes().as_slice())
    .execute(&f.pool)
    .await
    .unwrap();
    let tenant = TenantContext::resolved(f.community, &f.host);
    crate::handlers::side_effects::reconcile_channel_events(&tenant, &f.state)
        .await
        .unwrap();
    let heads: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND kind=39000 AND deleted_at IS NULL",
    )
    .bind(f.community.as_uuid())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        heads, 1001,
        "every live channel must receive canonical metadata"
    );
    let archived = f.snapshot(Uuid::from_u128(1001), &["retained"]).await;
    assert!(archived
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["archived", "true"]));
    let deleted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND channel_id=$2 AND deleted_at IS NULL",
    )
    .bind(f.community.as_uuid())
    .bind(Uuid::from_u128(1002))
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        deleted, 0,
        "repair must not resurrect any deleted-channel discovery"
    );
}
