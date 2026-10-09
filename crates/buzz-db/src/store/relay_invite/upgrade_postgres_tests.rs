use super::*;
use uuid::Uuid;

async fn legacy_fixture() -> (PgPool, Vec<Uuid>) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    crate::migration::run_migrations_through(&pool, 60)
        .await
        .unwrap();
    let mut communities = Vec::new();
    for index in 0..4 {
        let community = Uuid::new_v4();
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community)
            .bind(format!("upgrade-{index}-{community}.example"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO relay_invites(community_id,token_hash,created_by,expires_at,created_at) VALUES($1,$2,$3,now()+interval '1 day',now()-interval '1 hour')")
            .bind(community).bind(vec![index as u8;32]).bind(hex::encode([1;32]))
            .execute(&pool).await.unwrap();
        if index != 3 {
            // Historical accepted ban after issuance, already lifted by upgrade.
            sqlx::query("INSERT INTO moderation_actions(community_id,actor_pubkey,action,target_pubkey) VALUES($1,$2,'ban',$3)")
                .bind(community).bind(vec![2u8;32]).bind(vec![1u8;32])
                .execute(&pool).await.unwrap();
        }
        if index == 1 || index == 2 {
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("SELECT set_config('buzz.deletion_executor_community',$1,true),set_config('buzz.deletion_fence_generation','7',true)")
                .bind(community.to_string()).execute(&mut *tx).await.unwrap();
            sqlx::query(
                "UPDATE communities SET deletion_state=$2,deletion_fence_generation=7 WHERE id=$1",
            )
            .bind(community)
            .bind(if index == 1 { "quiescing" } else { "tombstone" })
            .execute(&mut *tx)
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        communities.push(community);
    }
    (pool, communities)
}

async fn assert_repaired_and_fenced(pool: &PgPool, communities: &[Uuid]) {
    for (index, community) in communities.iter().enumerate() {
        let revoked: bool = sqlx::query_scalar(
            "SELECT revoked_at IS NOT NULL FROM relay_invites WHERE community_id=$1",
        )
        .bind(community)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(revoked, index != 3, "tenant {index}");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT backfill_invite_revocations()")
            .fetch_one(pool)
            .await
            .unwrap(),
        0,
        "repair is idempotent"
    );
    // The repair restores context; ordinary writes stay fenced afterward.
    let error = sqlx::query("UPDATE relay_invites SET use_count=use_count+1 WHERE community_id=$1")
        .bind(communities[1])
        .execute(pool)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("community write fenced"),
        "{error}"
    );
    let triggers: i64=sqlx::query_scalar("SELECT count(*) FROM pg_trigger WHERE NOT tgisinternal AND tgname IN ('invite_restriction_lock','invite_issuer_revocation','invite_owner_lock','invite_owner_revocation','community_write_fence_relay_invites')")
        .fetch_one(pool).await.unwrap();
    assert_eq!(triggers, 5);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_backfill_repairs_history_in_active_and_deletion_fenced_tenants() {
    let (pool, communities) = legacy_fixture().await;
    crate::migration::run_migrations(&pool).await.unwrap();
    assert_repaired_and_fenced(&pool, &communities).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_desired_upgrade_reconciles_the_same_populated_history() {
    let (pool, communities) = legacy_fixture().await;
    // Apply the actual desired-state function/trigger sources to the populated
    // old schema, then the actual pgschema reconciliation entry point.
    sqlx::raw_sql("ALTER TABLE relay_invites ADD COLUMN revoked_at TIMESTAMPTZ")
        .execute(&pool)
        .await
        .unwrap();
    for sql in [
        include_str!("../../../../../schema/functions/invite_admission_lock.sql"),
        include_str!("../../../../../schema/functions/lock_invite_restrictions.sql"),
        include_str!("../../../../../schema/functions/revoke_banned_issuer_invites.sql"),
        include_str!("../../../../../schema/functions/revoke_restricted_agent_invites.sql"),
        include_str!("../../../../../schema/functions/backfill_invite_revocations.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    sqlx::raw_sql("CREATE TRIGGER invite_restriction_lock BEFORE INSERT OR UPDATE ON community_bans FOR EACH ROW EXECUTE FUNCTION lock_invite_restrictions(); CREATE TRIGGER invite_issuer_revocation AFTER INSERT OR UPDATE ON community_bans FOR EACH ROW EXECUTE FUNCTION revoke_banned_issuer_invites(); CREATE TRIGGER invite_owner_lock BEFORE INSERT OR UPDATE OF agent_owner_pubkey ON users FOR EACH ROW EXECUTE FUNCTION lock_invite_restrictions(); CREATE TRIGGER invite_owner_revocation AFTER INSERT OR UPDATE OF agent_owner_pubkey ON users FOR EACH ROW EXECUTE FUNCTION revoke_restricted_agent_invites();")
        .execute(&pool).await.unwrap();
    let still_unrepaired: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_invites WHERE revoked_at IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(still_unrepaired, 4, "DDL alone cannot repair history");
    sqlx::raw_sql(include_str!(
        "../../../../../scripts/reconcile-schema-after-pgschema.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_repaired_and_fenced(&pool, &communities).await;
}
