use super::*;
use buzz_core::CommunityId;
use nostr::JsonUtil;

async fn saved_lists(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY community_id,kind,channel_id,created_at,id),'[]') FROM events e WHERE kind IN (39000,39001,39002)")
        .fetch_one(pool).await.unwrap()
}

async fn seed_lists(f: &Fixture) {
    // Include a DM (its metadata contains members), an admin removal, an empty
    // active roster, and the relay itself in a roster (self-tagging).
    sqlx::query("UPDATE channels SET channel_type='dm' WHERE name='fixture-1'")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE channel_members SET role='owner' WHERE pubkey=decode($1,'hex')")
        .bind(TARGET)
        .execute(&f.pool)
        .await
        .unwrap();
    let signer = nostr::Keys::parse(&format!("{:064x}", 1)).unwrap();
    let channels: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT community_id,id FROM channels ORDER BY community_id,id")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    for (community, channel) in channels {
        let mut tx = crate::begin_community_event_write_transaction(
            &f.pool,
            CommunityId::from_uuid(community),
            crate::observability::WriterOperation::EventWrite,
        )
        .await
        .unwrap();
        for kind in [39000, 39001, 39002] {
            crate::replaceable::take_replacement_lock(
                &mut tx,
                kind,
                &signer.public_key().to_bytes(),
                Some(channel),
            )
            .await
            .unwrap();
        }
        crate::channel_members::acquire_channel_membership_lock(
            tx.conn(),
            CommunityId::from_uuid(community),
            channel,
        )
        .await
        .unwrap();
        memberships::rebuild(&mut tx, channel, &signer)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
}

async fn assert_current_lists(f: &Fixture) {
    let rows: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',encode(id,'hex'),'pubkey',encode(pubkey,'hex'),'created_at',extract(epoch FROM created_at)::bigint,'kind',kind,'tags',tags,'content',content,'sig',encode(sig,'hex')) FROM events WHERE community_id=$1 AND kind IN (39000,39001,39002) AND deleted_at IS NULL ORDER BY channel_id,kind")
        .bind(f.options.community).fetch_all(&f.pool).await.unwrap();
    assert_eq!(rows.len(), 9);
    for row in rows {
        let event = nostr::Event::from_json(row.to_string()).unwrap();
        event.verify().unwrap();
        let channel = Uuid::parse_str(event.tags.identifier().unwrap()).unwrap();
        let roster: Vec<(String,String)> = sqlx::query_as("SELECT encode(pubkey,'hex'),role::text FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND removed_at IS NULL ORDER BY pubkey")
            .bind(f.options.community).bind(channel).fetch_all(&f.pool).await.unwrap();
        let kind = row["kind"].as_i64().unwrap();
        let expected: BTreeSet<_> = roster
            .into_iter()
            .filter(|(_, role)| kind != 39001 || matches!(role.as_str(), "owner" | "admin"))
            .map(|(key, role)| {
                if kind == 39002 {
                    json!(["p", key, "", role]).to_string()
                } else if kind == 39001 {
                    json!(["p", key, role]).to_string()
                } else {
                    json!(["p", key]).to_string()
                }
            })
            .collect();
        let actual: BTreeSet<_> = row["tags"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t[0] == "p")
            .map(Value::to_string)
            .collect();
        if kind != 39000
            || row["tags"]
                .as_array()
                .unwrap()
                .contains(&json!(["t", "dm"]))
        {
            assert_eq!(actual, expected);
        } else {
            assert!(actual.is_empty());
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn signed_lists_rollback_on_write_failure_then_resume_without_changing_other_community() {
    journey().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_signed_lists_rollback_and_resume() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    Db::from_pool(pool.clone()).migrate().await.unwrap();
    pool.close().await;
    journey().await;
}

async fn journey() {
    let mut f = Fixture::new().await;
    seed_lists(&f).await;
    let before = f.snapshot().await;
    let lists = saved_lists(&f.pool).await;
    f.run(ExecutionMode::ReadOnly, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(f.snapshot().await, before);
    assert_eq!(saved_lists(&f.pool).await, lists);
    // Neither missing nor wrong key may delete even an earlier token batch.
    for signer in [
        None,
        Some(nostr::Keys::parse(&format!("{:064x}", 2)).unwrap()),
    ] {
        let mut store = Db::from_pool(f.pool.clone()).user_deletion_store();
        if let Some(signer) = signer {
            store = store.with_account_snapshot_signer(signer);
        }
        assert!(store
            .run_account_batches(&f.options, ExecutionMode::Delete, &mut Vec::new(), None)
            .await
            .is_err());
        assert_eq!(f.snapshot().await, before);
    }
    // Fail the LAST list insert after membership removal and two successful
    // list writes on that transaction. No test-only branch in production.
    sqlx::query("ALTER TABLE events ADD CONSTRAINT account_test_reject_members CHECK (kind<>39002) NOT VALID")
        .execute(&f.pool).await.unwrap();
    assert!(f
        .run(ExecutionMode::Delete, &mut Vec::new())
        .await
        .unwrap_err()
        .to_string()
        .contains("account_test_reject_members"));
    assert_eq!(
        f.snapshot().await["channel_members"],
        before["channel_members"]
    );
    assert_eq!(saved_lists(&f.pool).await, lists);
    sqlx::query("ALTER TABLE events DROP CONSTRAINT account_test_reject_members")
        .execute(&f.pool)
        .await
        .unwrap();
    // Interrupt the process while its last list is blocked, after the other
    // list replacements and membership DELETE ran but before COMMIT.
    let mut blocker = f.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM events WHERE community_id=$1 AND kind=39002 AND deleted_at IS NULL FOR UPDATE")
        .bind(f.options.community).fetch_all(&mut *blocker).await.unwrap();
    let store = f.store();
    let options = AccountBatchOptions {
        community: f.options.community,
        pubkey: TARGET.into(),
        batch_size: 2,
        max_items: 1000,
    };
    let task = tokio::spawn(async move {
        store
            .run_account_batches(&options, ExecutionMode::Delete, &mut Vec::new(), None)
            .await
    });
    let reached = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)))")
                .bind(pid).fetch_one(&f.pool).await.unwrap();
            if blocked { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await;
    task.abort();
    let stopped = task.await;
    blocker.rollback().await.unwrap();
    reached.unwrap();
    assert!(stopped.unwrap_err().is_cancelled());
    assert_eq!(
        f.snapshot().await["channel_members"],
        before["channel_members"]
    );
    assert_eq!(saved_lists(&f.pool).await, lists);
    // One channel commits, then the command stops. Its lists must already agree
    // with its table; channels not yet processed still agree with theirs.
    f.options.max_items = 1;
    let partial = f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    assert_eq!(partial.items, 1);
    assert!(!partial.supported_scope_exhausted);
    assert_current_lists(&f).await;
    f.options.max_items = 1000;
    f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    assert_current_lists(&f).await;
    let after = saved_lists(&f.pool).await;
    let other = |v: &Value| {
        v.as_array()
            .unwrap()
            .iter()
            .filter(|r| r["community_id"] == f.other.to_string())
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(other(&after), other(&lists));
    assert!(after
        .as_array()
        .unwrap()
        .iter()
        .filter(
            |r| r["community_id"] == f.options.community.to_string() && r["deleted_at"].is_null()
        )
        .all(|r| !r["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t[0] == "p" && t[1] == TARGET)));
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    assert_eq!(saved_lists(&f.pool).await, after);
    // Old channel deletion leaves member rows. Account cleanup must finish
    // without bringing that channel's discovery lists back to life.
    let deleted: Uuid=sqlx::query_scalar("UPDATE channels SET deleted_at=now() WHERE community_id=$1 AND name='fixture-3' RETURNING id")
        .bind(f.other).fetch_one(&f.pool).await.unwrap();
    f.options.community = f.other;
    f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    let heads: i64=sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND channel_id=$2 AND kind IN (39000,39001,39002) AND deleted_at IS NULL")
        .bind(f.other).bind(deleted).fetch_one(&f.pool).await.unwrap();
    assert_eq!(heads, 0);
}
