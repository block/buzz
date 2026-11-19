use super::*;
use buzz_core::CommunityId;
use nostr::JsonUtil;

async fn rosters(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY community_id,created_at,id),'[]') FROM events e WHERE kind=13534")
        .fetch_one(pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_only_membership_refresh_is_atomic_and_preserves_other_community() {
    let f = Fixture::new().await;
    // Community membership alone must require the signing key.
    sqlx::query("DELETE FROM channel_members WHERE community_id=$1 AND pubkey=decode($2,'hex')")
        .bind(f.options.community)
        .bind(TARGET)
        .execute(&f.pool)
        .await
        .unwrap();
    let signer = nostr::Keys::parse(&format!("{:064x}", 1)).unwrap();
    for community in [f.options.community, f.other] {
        let mut tx = crate::begin_community_event_write_transaction(
            &f.pool,
            CommunityId::from_uuid(community),
            crate::observability::WriterOperation::EventWrite,
        )
        .await
        .unwrap();
        crate::relay_members::write_nip43_membership_snapshot(&mut tx, &signer)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let before = f.snapshot().await;
    let old_rosters = rosters(&f.pool).await;
    let unsigned = Db::from_pool(f.pool.clone()).user_deletion_store();
    unsigned
        .run_account_batches(&f.options, ExecutionMode::ReadOnly, &mut Vec::new(), None)
        .await
        .unwrap();
    for store in [
        unsigned,
        Db::from_pool(f.pool.clone())
            .user_deletion_store()
            .with_account_snapshot_signer(nostr::Keys::parse(&format!("{:064x}", 2)).unwrap()),
    ] {
        assert!(store
            .run_account_batches(&f.options, ExecutionMode::Delete, &mut Vec::new(), None)
            .await
            .is_err());
        assert_eq!(f.snapshot().await, before);
        assert_eq!(rosters(&f.pool).await, old_rosters);
    }
    // Fail the replacement insert after the member DELETE and old-head retirement.
    sqlx::query("ALTER TABLE events ADD CONSTRAINT account_test_reject_community_roster CHECK (kind<>13534) NOT VALID")
        .execute(&f.pool).await.unwrap();
    let error = f
        .run(ExecutionMode::Delete, &mut Vec::new())
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("account_test_reject_community_roster"));
    assert_eq!(f.snapshot().await["relay_members"], before["relay_members"]);
    assert_eq!(rosters(&f.pool).await, old_rosters);
    sqlx::query("ALTER TABLE events DROP CONSTRAINT account_test_reject_community_roster")
        .execute(&f.pool)
        .await
        .unwrap();
    let mut output = Vec::new();
    f.run(ExecutionMode::Delete, &mut output).await.unwrap();
    assert!(records(&output)
        .iter()
        .any(|r| r["saved_community_roster"] == true));
    let current: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',encode(id,'hex'),'pubkey',encode(pubkey,'hex'),'created_at',extract(epoch FROM created_at)::bigint,'kind',kind,'tags',tags,'content',content,'sig',encode(sig,'hex')) FROM events WHERE community_id=$1 AND kind=13534 AND deleted_at IS NULL")
        .bind(f.options.community).fetch_all(&f.pool).await.unwrap();
    assert_eq!(current.len(), 1);
    nostr::Event::from_json(current[0].to_string())
        .unwrap()
        .verify()
        .unwrap();
    let expected: Vec<(String, String)> = sqlx::query_as(
        "SELECT pubkey,role::text FROM relay_members WHERE community_id=$1 ORDER BY pubkey",
    )
    .bind(f.options.community)
    .fetch_all(&f.pool)
    .await
    .unwrap();
    let actual: BTreeSet<_> = current[0]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tag| tag[0] == "member")
        .map(Value::to_string)
        .collect();
    assert_eq!(
        actual,
        expected
            .into_iter()
            .map(|(key, role)| json!(["member", key, role]).to_string())
            .collect()
    );
    assert!(!current[0]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tag| tag[1] == TARGET));
    let after = rosters(&f.pool).await;
    let other = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .filter(|r| r["community_id"] == f.other.to_string())
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(other(&after), other(&old_rosters));
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    assert_eq!(rosters(&f.pool).await, after);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_delete_removes_acceptance_recorded_after_acceptance_batch() {
    let f = Fixture::new().await;
    let community = f.options.community;
    // No foreign key cascades, so only the membership transaction can remove
    // an acceptance that commits after the acceptance batch ran.

    // An acceptance write in flight: the member row is key-share locked and
    // the acceptance is not yet visible to the acceptance batch.
    let mut holder = f.pool.begin().await.unwrap();
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .unwrap();
    crate::store::reference_check::require_relay_member(
        &mut holder,
        CommunityId::from_uuid(community),
        TARGET,
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
         VALUES ($1, $2, $3)",
    )
    .bind(community)
    .bind(TARGET)
    .bind("f".repeat(64))
    .execute(&mut *holder)
    .await
    .unwrap();

    let store = f.store();
    let options = AccountBatchOptions {
        community,
        pubkey: TARGET.to_owned(),
        batch_size: f.options.batch_size,
        max_items: f.options.max_items,
    };
    let run = tokio::spawn(async move {
        store
            .run_account_batches(&options, ExecutionMode::Delete, &mut Vec::new(), None)
            .await
    });
    // The membership delete waits on the member row lock.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(holder_pid)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            if blocked {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("membership delete waits on the acceptance writer");
    holder.commit().await.unwrap();
    run.await.unwrap().unwrap();

    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM join_policy_acceptances WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community)
    .bind(TARGET)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(left, 0, "acceptance left without its member");
    let member: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM relay_members WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community)
    .bind(TARGET)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(member, 0);
}
