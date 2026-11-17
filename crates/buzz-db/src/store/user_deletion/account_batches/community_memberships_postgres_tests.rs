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
