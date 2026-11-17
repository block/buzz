//! `Db::get_readable_named_only_channels_routed` reads the bounded replica
//! arm and fails closed to the writer. A different channel on each pool
//! shows which pool answered.

use super::*;

async fn seed_listed_system_channel(pool: &PgPool, community: Uuid, channel: Uuid, name: &str) {
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
         VALUES ($1, $2, $3, 'system', 'open', $4)",
    )
    .bind(channel)
    .bind(community)
    .bind(name)
    .bind(nostr::Keys::generate().public_key().to_bytes().to_vec())
    .execute(pool)
    .await
    .expect("insert system channel");
}

async fn answer(db: &Db, cid: CommunityId, reader: &[u8]) -> Vec<Uuid> {
    db.get_readable_named_only_channels_routed(cid, reader, &[])
        .await
        .expect("readable named-only channels")
        .into_iter()
        .map(|channel| channel.id)
        .collect()
}

/// Budget unset ⇒ writer; budget set and a fresh proved entry ⇒ replica;
/// an entry older than the budget ⇒ writer.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn readable_named_only_channels_routed_reads_the_bounded_replica() {
    let (writer, _writer_db) = ScratchDb::migrated(&admin_url().await, "named_w").await;
    let (replica, _replica_db) = ScratchDb::migrated(&admin_url().await, "named_r").await;

    let community = Uuid::new_v4();
    for pool in [&writer, &replica] {
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("named-only-routing-{}.example", community.simple()))
            .execute(pool)
            .await
            .expect("insert community");
    }
    let writer_only = Uuid::new_v4();
    let replica_only = Uuid::new_v4();
    seed_listed_system_channel(&writer, community, writer_only, "writer-system").await;
    seed_listed_system_channel(&replica, community, replica_only, "replica-system").await;

    let cid = CommunityId::from_uuid(community);
    let reader = nostr::Keys::generate().public_key().to_bytes().to_vec();
    let mut db = Db::from_pools(writer.clone(), replica.clone());
    db.fence().force_open_for_tests(chrono::Utc::now());

    assert_eq!(
        answer(&db, cid, &reader).await,
        vec![writer_only],
        "budget unset must answer from the writer"
    );

    db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
    assert_eq!(
        answer(&db, cid, &reader).await,
        vec![replica_only],
        "budget set and a fresh entry must answer from the replica"
    );

    db.fence().close();
    db.fence().force_open_for_tests_at(
        chrono::Utc::now(),
        std::time::Instant::now() - std::time::Duration::from_secs(10),
    );
    assert_eq!(
        answer(&db, cid, &reader).await,
        vec![writer_only],
        "an over-budget entry must fail closed to the writer"
    );
}
