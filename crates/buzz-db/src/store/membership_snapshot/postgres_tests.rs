use super::*;
use nostr::Keys;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;

async fn fixture(count: usize) -> (Db, PgPool, CommunityId, Keys, Vec<Uuid>) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community(&format!("membership-{}.local", Uuid::new_v4().simple()))
        .await
        .unwrap()
        .id;
    let viewer = Keys::generate();
    let ids: Vec<Uuid> = (0..count).map(|_| Uuid::new_v4()).collect();
    let mut tx = crate::begin_community_write_transaction(
        &pool,
        community,
        observability::WriterOperation::EventWrite,
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channels (community_id,id,name,created_by)
        SELECT $1,id,id::text,$3 FROM unnest($2::uuid[]) id",
    )
    .bind(community.as_uuid())
    .bind(&ids)
    .bind(viewer.public_key().to_bytes().to_vec())
    .execute(tx.conn())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channel_members (community_id,channel_id,pubkey,role)
        SELECT $1,id,$3,'owner' FROM unnest($2::uuid[]) id",
    )
    .bind(community.as_uuid())
    .bind(&ids)
    .bind(viewer.public_key().to_bytes().to_vec())
    .execute(tx.conn())
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (db, pool, community, viewer, ids)
}

fn all_types() -> Vec<String> {
    CHANNEL_TYPES.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_snapshot_scope_roles_types_archive_hidden_and_overflow() {
    let (db, pool, community, viewer, ids) = fixture(8).await;
    for (i, (kind, role)) in CHANNEL_TYPES
        .iter()
        .zip(["owner", "admin", "member", "bot", "guest"])
        .enumerate()
    {
        sqlx::query(
            "UPDATE channels SET channel_type=$3::channel_type, visibility='private',
            archived_at=CASE WHEN $4 THEN now() ELSE NULL END WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(ids[i])
        .bind(kind)
        .bind(i == 1)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE channel_members SET role=$3::member_role, hidden_at=now()
            WHERE community_id=$1 AND channel_id=$2",
        )
        .bind(community.as_uuid())
        .bind(ids[i])
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE channels SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(ids[5])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(community.as_uuid())
    .bind(ids[6])
    .execute(&pool)
    .await
    .unwrap();
    // Open but not a member is excluded as well.
    sqlx::query("DELETE FROM channel_members WHERE community_id=$1 AND channel_id=$2")
        .bind(community.as_uuid())
        .bind(ids[7])
        .execute(&pool)
        .await
        .unwrap();
    let dead = PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/absent")
        .unwrap();
    dead.close().await;
    let routed = Db::from_pools(pool.clone(), dead);
    let cut = routed
        .membership_snapshot(community, &viewer.public_key(), &all_types())
        .await
        .unwrap();
    assert_eq!(cut.channels.len(), 5);
    assert!(cut.channels.windows(2).all(|p| p[0].id < p[1].id));
    for (i, role) in ["owner", "admin", "member", "bot", "guest"]
        .iter()
        .enumerate()
    {
        let row = cut.channels.iter().find(|c| c.id == ids[i]).unwrap();
        assert_eq!(&row.role, role);
        assert_eq!(row.channel_type, CHANNEL_TYPES[i]);
        assert_eq!(row.visibility, "private");
        assert_eq!(row.archived, i == 1);
    }
    let systems = db
        .membership_snapshot(community, &viewer.public_key(), &["system".into()])
        .await
        .unwrap();
    assert_eq!(systems.channels.len(), 1);
    assert_ne!(cut.snapshot_id, systems.snapshot_id);
    let other = db
        .membership_snapshot(community, &Keys::generate().public_key(), &all_types())
        .await
        .unwrap();
    assert!(other.channels.is_empty());
    let other_tenant = db
        .ensure_configured_community(&format!("other-{}.local", Uuid::new_v4().simple()))
        .await
        .unwrap()
        .id;
    assert!(db
        .membership_snapshot(other_tenant, &viewer.public_key(), &all_types())
        .await
        .unwrap()
        .channels
        .is_empty());
    assert!(db
        .membership_snapshot(community, &viewer.public_key(), &["unknown".into()])
        .await
        .is_err());
    let (db, _, community, viewer, _) = fixture(MAX_CHANNELS + 1).await;
    assert!(matches!(
        db.membership_snapshot(community, &viewer.public_key(), &all_types())
            .await,
        Err(DbError::MembershipSnapshotTooLarge)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_snapshot_468_channels_size_independent_of_other_members_and_index_plan() {
    let (db, pool, community, viewer, ids) = fixture(468).await;
    let before = db
        .membership_snapshot(community, &viewer.public_key(), &all_types())
        .await
        .unwrap();
    let bytes = serde_json::to_vec(&before.channels).unwrap().len();
    for (i, id) in ids.iter().enumerate() {
        let members = [5898, 5847, 5307, 3314, 2007].get(i).copied().unwrap_or(5);
        sqlx::query(
            "INSERT INTO channel_members (community_id,channel_id,pubkey)
            SELECT $1,$2,decode(md5(n::text)||md5(n::text),'hex') FROM generate_series(1,$3) n",
        )
        .bind(community.as_uuid())
        .bind(id)
        .bind(members - 1)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::raw_sql("ANALYZE channel_members; ANALYZE channels")
        .execute(&pool)
        .await
        .unwrap();
    let start = std::time::Instant::now();
    let after = db
        .membership_snapshot(community, &viewer.public_key(), &all_types())
        .await
        .unwrap();
    assert_eq!(before.channels, after.channels);
    assert_eq!(before.snapshot_id, after.snapshot_id);
    assert_eq!(bytes, serde_json::to_vec(&after.channels).unwrap().len());
    let plan = sqlx::query(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (ANALYZE, BUFFERS) {SNAPSHOT_SQL}"
    )))
    .bind(community.as_uuid())
    .bind(viewer.public_key().to_bytes().to_vec())
    .bind(all_types())
    .bind((MAX_CHANNELS + 1) as i64)
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.get::<String, _>(0))
    .collect::<Vec<_>>()
    .join("\n");
    assert!(plan.contains("idx_channel_members_pubkey"), "{plan}");
    println!(
        "468-channel projection: bytes={bytes}, query_ms={}\n{plan}",
        start.elapsed().as_secs_f64() * 1000.0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cluster_global_membership_snapshot_role_and_removal_commit_during_statement_keep_one_cut()
{
    let (db, pool, community, viewer, ids) = fixture(4).await;
    let before = db
        .membership_snapshot(community, &viewer.public_key(), &all_types())
        .await
        .unwrap();
    // A private view pauses the REAL production query without changing its SQL.
    // The gate runs under its statement snapshot while public canonical rows change.
    let schema = format!("membership_gate_{}", Uuid::new_v4().simple());
    let lock_key = i64::from_be_bytes(Uuid::new_v4().as_bytes()[..8].try_into().unwrap());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema};
        CREATE FUNCTION {schema}.gate() RETURNS boolean LANGUAGE plpgsql VOLATILE AS
        $$ BEGIN PERFORM pg_advisory_xact_lock({lock_key}); RETURN true; END $$;
        CREATE VIEW {schema}.channel_members AS SELECT cm.* FROM public.channel_members cm WHERE {schema}.gate()")))
        .execute(&pool).await.unwrap();
    let application = schema.clone();
    let path = schema.clone();
    let isolated = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |c, _| {
            let setup = format!(
                "SET search_path TO {path},pg_catalog,public; SET application_name='{path}'"
            );
            Box::pin(async move {
                sqlx::raw_sql(sqlx::AssertSqlSafe(setup)).execute(c).await?;
                Ok(())
            })
        })
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let snapshot_db = Db::from_pool(isolated);
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock_key)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let reader = snapshot_db.clone();
    let pubkey = viewer.public_key();
    let pending = tokio::spawn(async move {
        reader
            .membership_snapshot(community, &pubkey, &all_types())
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity
                WHERE application_name=$1 AND wait_event='advisory')",
            )
            .bind(&application)
            .fetch_one(&pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("production projection must reach statement gate");
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE channel_members SET role='guest' WHERE community_id=$1 AND channel_id=$2")
        .bind(community.as_uuid())
        .bind(ids[0])
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(community.as_uuid())
    .bind(ids[1])
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE channels SET archived_at=now() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(ids[2])
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE channels SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(ids[3])
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock_key)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let cut = pending.await.unwrap().unwrap();
    assert_eq!(cut.channels, before.channels);
    assert_eq!(cut.snapshot_id, before.snapshot_id);
    let next = snapshot_db
        .membership_snapshot(community, &viewer.public_key(), &all_types())
        .await
        .unwrap();
    assert_eq!(next.channels.len(), 2);
    assert_eq!(
        next.channels.iter().find(|c| c.id == ids[0]).unwrap().role,
        "guest"
    );
    assert!(
        next.channels
            .iter()
            .find(|c| c.id == ids[2])
            .unwrap()
            .archived
    );
    assert!(!next
        .channels
        .iter()
        .any(|c| c.id == ids[1] || c.id == ids[3]));
    assert_ne!(next.snapshot_id, cut.snapshot_id);
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&pool)
        .await
        .unwrap();
}
