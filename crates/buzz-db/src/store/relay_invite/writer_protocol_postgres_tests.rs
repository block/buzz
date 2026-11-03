use super::revocation_postgres_tests::{
    claim_relay_invite_outcome, fixture, named_pool, wait_for_lock,
};
use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn authorization_owner_writer_waits_before_claim_owner_row_and_links_once() {
    let (pool, community, _, claimant) = fixture().await;
    let owner = hex::encode([4; 32]);
    for key in [[2; 32], [4; 32]] {
        crate::user::ensure_user(&pool, community, &key)
            .await
            .unwrap();
    }
    let mut claim = pool.begin().await.unwrap();
    lock_admission(&mut claim, community).await.unwrap();
    let (writer_pool, name) = named_pool().await;
    let setter = tokio::spawn(async move {
        Db::from_pool(writer_pool)
            .set_agent_owner_for_authorization(community, &[2; 32], &[4; 32])
            .await
    });
    wait_for_lock(&pool, &name, "advisory", &setter).await;
    // Exercise the production claim persistence while the setter is queued.
    // If the setter locks the user first, this bounded operation cannot finish.
    let written = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        super::claim_owner::persist_claim_owner(&mut claim, community, &claimant, Some(&owner)),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(written.link_created);
    claim.commit().await.unwrap();
    assert!(
        !tokio::time::timeout(std::time::Duration::from_secs(5), setter)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    );
    assert_eq!(
        crate::user::get_agent_channel_policy(&pool, community, &[2; 32])
            .await
            .unwrap()
            .unwrap()
            .1,
        Some(vec![4; 32])
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn ensure_user_insert_and_conflict_do_not_take_admission_lock() {
    let (pool, community, _, _) = fixture().await;
    let mut claim = pool.begin().await.unwrap();
    lock_admission(&mut claim, community).await.unwrap();
    let (writer_pool, name) = named_pool().await;
    for expected in [true, false] {
        let mut table_lock = pool.begin().await.unwrap();
        sqlx::query("LOCK TABLE users IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *table_lock)
            .await
            .unwrap();
        let insert_pool = writer_pool.clone();
        let insert = tokio::spawn(async move {
            crate::user::ensure_user(&insert_pool, community, &[5; 32]).await
        });
        // Inspect the writer while its INSERT is in flight, not after its
        // autocommit releases locks. It must reach the table behind admission.
        wait_for_lock(&pool, &name, "relation", &insert).await;
        let locks: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid \
             WHERE a.application_name=$1 AND l.locktype='advisory'",
        )
        .bind(&name)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            locks, 0,
            "user insert must not hold or wait on advisory locks"
        );
        table_lock.commit().await.unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), insert)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            expected
        );
    }
    claim.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn linking_banned_owner_revokes_agent_invites_but_timeout_does_not() {
    for banned in [true, false] {
        let (pool, community, issuer, claimant) = fixture().await;
        for key in [[1; 32], [4; 32]] {
            crate::user::ensure_user(&pool, community, &key)
                .await
                .unwrap();
        }
        let invite = mint_relay_invite(&pool, community, &issuer, 3600, Some(1))
            .await
            .unwrap();
        if banned {
            crate::moderation::ban_member(&pool, community, &[4; 32], &[3; 32], None, None)
                .await
                .unwrap();
        } else {
            crate::moderation::timeout_member(
                &pool,
                community,
                &[4; 32],
                &[3; 32],
                Utc::now() + chrono::Duration::hours(1),
                None,
            )
            .await
            .unwrap();
        }
        assert!(
            crate::user::set_agent_owner(&pool, community, &[1; 32], &[4; 32])
                .await
                .unwrap()
        );
        let revoked: bool =
            sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM relay_invites WHERE id=$1")
                .bind(invite.invite_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(revoked, banned);
        if banned {
            crate::moderation::unban_member(&pool, community, &[4; 32], &[3; 32])
                .await
                .unwrap();
            assert_eq!(
                claim_relay_invite_outcome(
                    &pool,
                    community,
                    &hash_v2_code(&invite.code),
                    &claimant,
                    None
                )
                .await
                .unwrap(),
                ClaimOutcome::Revoked
            );
        } else {
            assert_eq!(
                claim_relay_invite_outcome(
                    &pool,
                    community,
                    &hash_v2_code(&invite.code),
                    &claimant,
                    None
                )
                .await
                .unwrap(),
                ClaimOutcome::IssuerRestricted
            );
            crate::moderation::untimeout_member(&pool, community, &[4; 32], &[3; 32])
                .await
                .unwrap();
            assert!(matches!(
                claim_relay_invite_outcome(
                    &pool,
                    community,
                    &hash_v2_code(&invite.code),
                    &claimant,
                    None
                )
                .await
                .unwrap(),
                ClaimOutcome::Joined { .. }
            ));
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn marker_writers_serialize_before_rows_and_commit_revocation_with_marker() {
    for timeout in [false, true] {
        let (pool, community, issuer, _) = fixture().await;
        let invite = mint_relay_invite(&pool, community, &issuer, 3600, None)
            .await
            .unwrap();
        for key in [[1; 32], [4; 32]] {
            crate::user::ensure_user(&pool, community, &key)
                .await
                .unwrap();
        }
        crate::user::set_agent_owner(&pool, community, &[4; 32], &[1; 32])
            .await
            .unwrap();
        let agent_invite = mint_relay_invite(&pool, community, &hex::encode([4; 32]), 3600, None)
            .await
            .unwrap();
        if timeout {
            // A timeout must also repair revocation for an existing active ban,
            // rather than revoking invites for timeout-only principals.
            crate::moderation::ban_member(&pool, community, &[1; 32], &[3; 32], None, None)
                .await
                .unwrap();
            sqlx::query("UPDATE relay_invites SET revoked_at=NULL WHERE id IN ($1,$2)")
                .bind(invite.invite_id)
                .bind(agent_invite.invite_id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let action = uuid::Uuid::new_v4();
        let lease = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO relay_admin_actions \
            (id, report_community_id, request_id, actor_pubkey, actor_role, action, state, \
             enforcement_target_pubkey, timeout_secs, timeout_until, action_lease_token, action_lease_expires_at) \
            VALUES ($1,$2,$1,$3,'operator',$4,'enforcing',$5, \
                    CASE WHEN $6 THEN 3600 END, CASE WHEN $6 THEN clock_timestamp()+interval '1 hour' END, \
                    $7,clock_timestamp()+interval '1 hour')")
            .bind(action).bind(community.as_uuid()).bind(vec![3u8;32])
            .bind(if timeout {"timeout"} else {"ban"}).bind(vec![1u8;32]).bind(timeout).bind(lease)
            .execute(&pool).await.unwrap();
        let mut admission = pool.begin().await.unwrap();
        lock_admission(&mut admission, community).await.unwrap();
        let (writer_pool, name) = named_pool().await;
        let writer = tokio::spawn(async move {
            if timeout {
                crate::relay_admin_actions::execute_timeout_with_marker(
                    &writer_pool,
                    action,
                    lease,
                    community,
                    &[1; 32],
                    &[3; 32],
                    Utc::now() + chrono::Duration::hours(1),
                    None,
                )
                .await
            } else {
                crate::relay_admin_actions::execute_ban_with_marker(
                    &writer_pool,
                    action,
                    lease,
                    community,
                    &[1; 32],
                    &[3; 32],
                    None,
                )
                .await
            }
        });
        wait_for_lock(&pool, &name, "advisory", &writer).await;
        // The action row must still be free while admission is held. This also
        // prevents future inversion with action-row locking.
        sqlx::query("SELECT id FROM relay_admin_actions WHERE id=$1 FOR UPDATE NOWAIT")
            .bind(action)
            .fetch_one(&mut *admission)
            .await
            .unwrap();
        admission.commit().await.unwrap();
        assert!(writer.await.unwrap().unwrap());
        let state: (bool, Option<String>) = sqlx::query_as(
            "SELECT i.revoked_at IS NOT NULL, a.step_marker FROM relay_invites i \
             JOIN relay_admin_actions a ON a.id=$2 WHERE i.id=$1",
        )
        .bind(invite.invite_id)
        .bind(action)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state, (true, Some("mutation_committed".into())));
        let agent_revoked: bool =
            sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM relay_invites WHERE id=$1")
                .bind(agent_invite.invite_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(agent_revoked, "marker ban also revokes owned-agent invites");
        // Already marked: revocation and domain mutations must roll back too.
        sqlx::query("UPDATE relay_invites SET revoked_at=NULL WHERE id IN ($1,$2)")
            .bind(invite.invite_id)
            .bind(agent_invite.invite_id)
            .execute(&pool)
            .await
            .unwrap();
        let repeated = if timeout {
            crate::relay_admin_actions::execute_timeout_with_marker(
                &pool,
                action,
                lease,
                community,
                &[1; 32],
                &[3; 32],
                Utc::now() + chrono::Duration::hours(1),
                None,
            )
            .await
        } else {
            crate::relay_admin_actions::execute_ban_with_marker(
                &pool, action, lease, community, &[1; 32], &[3; 32], None,
            )
            .await
        }
        .unwrap();
        assert!(!repeated);
        let revoked: bool =
            sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM relay_invites WHERE id=$1")
                .bind(invite.invite_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!revoked, "failed marker must not commit revocation");
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn linked_authorization_owner_and_missing_agent_do_not_wait_on_admission() {
    let (pool, community, _, _) = fixture().await;
    for key in [[2; 32], [4; 32]] {
        crate::user::ensure_user(&pool, community, &key)
            .await
            .unwrap();
    }
    assert!(
        crate::user::set_agent_owner(&pool, community, &[2; 32], &[4; 32])
            .await
            .unwrap()
    );
    let mut claim = pool.begin().await.unwrap();
    lock_admission(&mut claim, community).await.unwrap();
    let db = Db::from_pool(pool.clone());
    for owner in [[4; 32], [6; 32]] {
        assert!(!tokio::time::timeout(
            std::time::Duration::from_secs(5),
            db.set_agent_owner_for_authorization(community, &[2; 32], &owner)
        )
        .await
        .unwrap()
        .unwrap());
    }
    assert!(matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            db.set_agent_owner_for_authorization(community, &[7; 32], &[4; 32])
        )
        .await
        .unwrap(),
        Err(crate::DbError::NotFound(_))
    ));
    claim.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unknown_claim_code_is_rejected_without_waiting_on_admission() {
    let (pool, community, _, claimant) = fixture().await;
    let mut held = pool.begin().await.unwrap();
    lock_admission(&mut held, community).await.unwrap();
    // A forged code must not queue behind admission; if the claim takes the
    // lock before looking the code up, this bounded call cannot finish.
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        claim_relay_invite_outcome(&pool, community, &[0xAB; 32], &claimant, None),
    )
    .await
    .expect("unknown code waited on the admission lock")
    .unwrap();
    assert_eq!(outcome, ClaimOutcome::Invalid);
    held.rollback().await.unwrap();
}
