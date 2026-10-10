use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn activation_checks_labels_but_tolerates_unpublished_ordinary_metadata() {
    let (db, community) = database(false).await;
    let relay = Keys::generate();
    let owner = Keys::generate();
    // Page boundary: the final bad channel must not hide behind 128 good rows.
    for index in 1..=129u128 {
        let channel = Uuid::from_u128(index);
        let create = command(&owner, channel, 9007, &[("label", "retained")]);
        apply(&db, community, &relay, &create).await;
    }
    assert_eq!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await
            .unwrap(),
        129
    );
    let channel = Uuid::from_u128(129);
    db.set_topic(
        community,
        channel,
        "updated without publication",
        owner.public_key().as_bytes(),
    )
    .await
    .unwrap();
    db.archive_channel(community, channel).await.unwrap();
    assert_eq!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await
            .unwrap(),
        129,
        "unpublished topic/archive changes must not prevent routine restart"
    );
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert!(write.needs_snapshot().await.unwrap());
    snapshot(&mut write, &relay).await;
    write.commit().await.unwrap();
    assert_eq!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await
            .unwrap(),
        129
    );
    // A correctly signed but stale-label head is still incompatible.
    sqlx::query("UPDATE channels SET labels = ARRAY[]::text[] WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(channel)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(db
        .verify_channel_metadata_activation(relay.public_key())
        .await
        .is_err());
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    snapshot(&mut write, &relay).await;
    write.commit().await.unwrap();
    assert_eq!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await
            .unwrap(),
        129
    );
    // A second tenant is checked independently; the audit does not create its head.
    let other = db
        .ensure_configured_community("activation-other.example")
        .await
        .unwrap()
        .id;
    let missing = Uuid::new_v4();
    db.create_channel_with_id(
        other,
        missing,
        "missing",
        ChannelType::Stream,
        ChannelVisibility::Open,
        None,
        owner.public_key().as_bytes(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await
            .unwrap(),
        130
    );
    // Missing unlabeled publication is recoverable; missing labeled state is not.
    sqlx::query("UPDATE channels SET labels = ARRAY['retained']::text[] WHERE community_id = $1 AND id = $2")
        .bind(other.as_uuid()).bind(missing).execute(db.pool()).await.unwrap();
    assert!(db
        .verify_channel_metadata_activation(relay.public_key())
        .await
        .is_err());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND kind = 39000")
            .bind(other.as_uuid())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(
        count, 0,
        "activation must not silently repair incompatible state"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repair_pages_cover_the_catalog_without_crossing_tenants_or_tombstones() {
    let (db, community) = database(false).await;
    let other = db
        .ensure_configured_community("repair-other.example")
        .await
        .unwrap()
        .id;
    // Fixtures only: more than the interactive catalog cap, including one
    // archived row that repair must retain and one deleted row it must omit.
    sqlx::query(
        "INSERT INTO channels (community_id, id, name, created_by, archived_at, deleted_at) \
         SELECT $1, lpad(to_hex(n), 32, '0')::uuid, 'repair', decode(repeat('11', 32), 'hex'), \
         CASE WHEN n = 1001 THEN now() END, CASE WHEN n = 1002 THEN now() END \
         FROM generate_series(1, 1002) AS n",
    )
    .bind(community.as_uuid())
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channels (community_id, id, name, created_by) \
         VALUES ($1, $2, 'other', decode(repeat('11', 32), 'hex'))",
    )
    .bind(other.as_uuid())
    .bind(Uuid::from_u128(1003))
    .execute(db.pool())
    .await
    .unwrap();
    let mut cursor = Uuid::nil();
    let mut all = Vec::new();
    loop {
        let page = db
            .channel_metadata_repair_page(community, cursor)
            .await
            .unwrap();
        assert!(page.len() <= 128);
        if page.is_empty() {
            break;
        }
        assert!(page.iter().all(|id| *id > cursor));
        cursor = *page.last().unwrap();
        all.extend(page);
    }
    assert_eq!(all, (1..=1001).map(Uuid::from_u128).collect::<Vec<_>>());
}
