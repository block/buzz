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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn activation_skips_retiring_communities_but_checks_healthy_tenants() {
    let (db, healthy) = database(false).await;
    let relay = Keys::generate();
    let owner = Keys::generate();
    let channel = Uuid::new_v4();
    apply(
        &db,
        healthy,
        &relay,
        &command(&owner, channel, 9007, &[("label", "retained")]),
    )
    .await;
    for state in ["quiescing", "fenced", "tombstone"] {
        let community = db
            .ensure_configured_community(&format!("{state}.example"))
            .await
            .unwrap()
            .id;
        db.create_channel_with_id(
            community,
            channel,
            "retiring",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap();
        // A non-serving tenant is not an activation prerequisite, even if its
        // retained rows would fail canonical label validation.
        sqlx::query("UPDATE channels SET labels = ARRAY['INVALID']::text[] WHERE community_id=$1")
            .bind(community.as_uuid())
            .execute(db.pool())
            .await
            .unwrap();
        let mut lifecycle = db.pool().begin().await.unwrap();
        retire_community(&mut lifecycle, community, state).await;
        lifecycle.commit().await.unwrap();
    }
    assert_eq!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await
            .unwrap(),
        1
    );
    sqlx::query("UPDATE channels SET labels=ARRAY['drift']::text[] WHERE community_id=$1")
        .bind(healthy.as_uuid())
        .execute(db.pool())
        .await
        .unwrap();
    assert!(matches!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await,
        Err(crate::DbError::InvalidData(_))
    ));
    sqlx::query("UPDATE channels SET labels=ARRAY['INVALID']::text[] WHERE community_id=$1")
        .bind(healthy.as_uuid())
        .execute(db.pool())
        .await
        .unwrap();
    assert!(matches!(
        db.verify_channel_metadata_activation(relay.public_key())
            .await,
        Err(crate::DbError::InvalidData(_))
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn activation_skips_channels_retired_after_page_load_before_lock() {
    let (db, community) = database(false).await;
    let relay = Keys::generate();
    let owner = Keys::generate();
    let first = Uuid::from_u128(1);
    let labeled = Uuid::from_u128(2);
    let headless = Uuid::from_u128(3);
    let purged = Uuid::from_u128(4);
    for channel in [first, labeled] {
        apply(
            &db,
            community,
            &relay,
            &command(&owner, channel, 9007, &[("label", "retained")]),
        )
        .await;
    }
    for channel in [headless, purged] {
        db.create_channel_with_id(
            community,
            channel,
            "headless",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap();
    }
    let mut blocker = db
        .begin_channel_metadata_write(community, first, relay.public_key())
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(blocker.tx.conn())
        .await
        .unwrap();
    let retire = async {
        // The audit cannot request the first channel lock until its entire page
        // (including the three later channels) has been read.
        super::lifecycle_postgres_tests::blocked_pid(&db, pid).await;
        for channel in [labeled, headless] {
            assert!(db.soft_delete_channel(community, channel).await.unwrap());
        }
        sqlx::query("DELETE FROM channels WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(purged)
            .execute(db.pool())
            .await
            .unwrap();
        blocker.rollback().await.unwrap();
    };
    let (result, ()) = tokio::join!(
        db.verify_channel_metadata_activation(relay.public_key()),
        retire
    );
    assert_eq!(
        result.unwrap(),
        1,
        "retired channels are skipped, not counted as verified"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn activation_skips_community_retirement_between_enumeration_and_admission() {
    let (db, healthy) = database(false).await;
    let relay = Keys::generate();
    let owner = Keys::generate();
    let channel = Uuid::new_v4();
    apply(
        &db,
        healthy,
        &relay,
        &command(&owner, channel, 9007, &[("label", "retained")]),
    )
    .await;
    for state in ["quiescing", "fenced", "tombstone"] {
        let community = db
            .ensure_configured_community(&format!("race-{state}.example"))
            .await
            .unwrap()
            .id;
        apply(
            &db,
            community,
            &relay,
            &command(&owner, channel, 9007, &[("label", "retained")]),
        )
        .await;
        let mut blocker = db.pool().begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
            .bind(community.as_uuid())
            .execute(&mut *blocker)
            .await
            .unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let retire = async {
            super::lifecycle_postgres_tests::blocked_pid(&db, pid).await;
            // Match lifecycle's exclusive-lock/update/commit boundary.
            retire_community(&mut blocker, community, state).await;
            blocker.commit().await.unwrap();
        };
        let (result, ()) = tokio::join!(
            db.verify_channel_metadata_activation(relay.public_key()),
            retire
        );
        assert_eq!(result.unwrap(), 1, "healthy tenant survives {state} race");
    }
}

async fn retire_community(tx: &mut sqlx::PgConnection, community: CommunityId, state: &str) {
    sqlx::query("SELECT set_config('buzz.deletion_executor_community', $1, true), set_config('buzz.deletion_fence_generation', '0', true)")
        .bind(community.to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE communities SET deletion_state=$2, deleted_at=CASE WHEN $2='tombstone' THEN now() END WHERE id=$1")
        .bind(community.as_uuid()).bind(state).execute(tx).await.unwrap();
}
