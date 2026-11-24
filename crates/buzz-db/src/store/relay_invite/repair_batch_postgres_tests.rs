use super::upgrade_postgres_tests::{run_invite_repair_script, start_invite_repair_script};
use super::*;
use chrono::{DateTime, Utc};
use uuid::Uuid;

type InviteState = (Uuid, Uuid, Option<DateTime<Utc>>, bool);

async fn snapshot(pool: &PgPool) -> Vec<InviteState> {
    sqlx::query_as(
        "SELECT community_id, id, revoked_at, legacy_admission \
         FROM relay_invites ORDER BY community_id, id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

// Correctness/restart coverage for the real operator script. Scaling is checked
// separately with successor EXPLAIN ANALYZE plans and three-size timings; a
// correct final state alone cannot detect repeated prefix scans.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_repair_resumes_committed_batches_and_resets_cursor_for_each_community() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let first = Uuid::from_u128(1);
    let later = Uuid::from_u128(2);
    let fenced = Uuid::from_u128(3);
    for community in [first, later, fenced] {
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("repair-{community}.example"))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO users (community_id, pubkey) \
         VALUES ($1, decode(repeat('04',32),'hex')), ($1, decode(repeat('05',32),'hex'))",
    )
    .bind(first)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE users SET agent_owner_pubkey=decode(repeat('05',32),'hex') \
         WHERE community_id=$1 AND pubkey=decode(repeat('04',32),'hex')",
    )
    .bind(first)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO community_bans \
         (community_id, pubkey, actor_pubkey, banned, ban_expires_at) \
         SELECT $1, decode(repeat(key,32),'hex'), decode(repeat('03',32),'hex'), \
                key <> '05', CASE WHEN key='06' THEN now()-interval '1 hour' END \
         FROM unnest(ARRAY['01','05','06']) AS key",
    )
    .bind(first)
    .execute(&pool)
    .await
    .unwrap();
    // 1,204 candidates in one community: active bans, spared rows, malformed
    // issuers, owner history, expired bans, uppercase issuers, existing stamps.
    sqlx::query(
        "INSERT INTO relay_invites \
         (community_id, id, token_hash, created_by, expires_at, revoked_at) \
         SELECT $1, lpad(to_hex(i),32,'0')::uuid, decode(md5(i::text)||md5(i::text),'hex'), \
                CASE i%7 WHEN 0 THEN repeat('01',32) WHEN 1 THEN repeat('02',32) \
                  WHEN 2 THEN 'not-a-pubkey' WHEN 3 THEN repeat('04',32) \
                  WHEN 4 THEN repeat('06',32) WHEN 5 THEN repeat('AB',32) \
                  ELSE repeat('02',32) END, now()+interval '1 day', \
                CASE WHEN i%7=6 THEN '2020-01-01 UTC'::timestamptz END \
         FROM generate_series(0,1203) AS i",
    )
    .bind(first)
    .execute(&pool)
    .await
    .unwrap();
    // Native active bans still revoke; expired-ban history alone does not.
    for (id, issuer) in [(2001, "01"), (2002, "06")] {
        sqlx::query(
            "INSERT INTO relay_invites \
             (community_id,id,token_hash,created_by,expires_at,legacy_admission) \
             VALUES ($1,$2,$3,$4,now()+interval '1 day',false)",
        )
        .bind(first)
        .bind(Uuid::from_u128(id))
        .bind([id as u8; 32].as_slice())
        .bind(issuer.repeat(32))
        .execute(&pool)
        .await
        .unwrap();
    }
    // Later community's nil ID sorts below the first community's final cursor.
    // A separate fenced community must remain untouched, including its marker.
    for community in [later, fenced] {
        sqlx::query(
            "INSERT INTO relay_invites (community_id,id,token_hash,created_by,expires_at) \
             VALUES ($1,$2,$3,'malformed',now()+interval '1 day')",
        )
        .bind(community)
        .bind(Uuid::nil())
        .bind([7u8; 32].as_slice())
        .execute(&pool)
        .await
        .unwrap();
    }
    super::upgrade_postgres_tests::set_deletion_state(&pool, fenced, "quiescing").await;
    let before = snapshot(&pool).await;

    // The 501st candidate blocks the second batch, after exactly 500 commits.
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM relay_invites WHERE community_id=$1 AND id=$2 FOR UPDATE")
        .bind(first)
        .bind(Uuid::from_u128(500))
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let name = format!("invite-repair-{}", Uuid::new_v4());
    let child = start_invite_repair_script(Some(&name)).await;
    let repair = tokio::spawn(async move { child.wait_with_output().await });
    super::revocation_postgres_tests::wait_for_lock(&pool, &name, "transactionid", &repair).await;
    let committed = snapshot(&pool).await;
    assert_eq!(committed.iter().filter(|row| !row.3).count(), 502);
    // Cancel the real backend, not a test-only branch in the maintenance SQL.
    let cancelled: bool = sqlx::query_scalar(
        "SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE application_name=$1",
    )
    .bind(&name)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(cancelled);
    let output = repair.await.unwrap().unwrap();
    assert!(!output.status.success(), "interrupted script must fail");
    assert_eq!(
        snapshot(&pool).await,
        committed,
        "only the first batch committed"
    );
    blocker.rollback().await.unwrap();

    run_invite_repair_script().await;
    let repaired = snapshot(&pool).await;
    for (index, row) in repaired.iter().enumerate() {
        let (community, id, revoked_at, legacy) = row;
        if *community == fenced {
            assert_eq!(row, &before[index], "fenced row remains unchanged");
            continue;
        }
        assert!(!legacy, "all active-community markers cleared");
        let n = id.as_u128();
        let should_revoke = *community == later || (n < 1204 && n % 7 != 1) || n == 2001;
        assert_eq!(
            revoked_at.is_some(),
            should_revoke,
            "invite {community}/{id}"
        );
        if let Some(stamp) = before[index].2 {
            assert_eq!(*revoked_at, Some(stamp), "pre-existing stamp preserved");
        }
        if !committed[index].3 {
            assert_eq!(row, &committed[index], "committed prefix remains unchanged");
        }
    }
    run_invite_repair_script().await;
    assert_eq!(snapshot(&pool).await, repaired, "second run is idempotent");
}
