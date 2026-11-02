use super::*;
use sqlx::PgPool;
use std::time::Duration;

async fn pool() -> PgPool {
    PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap()
}

async fn request(pool: &PgPool) -> EntityLease {
    let db = crate::Db::from_pool(pool.clone());
    let request = db
        .user_deletion_store()
        .submit(
            &format!("demo:{}", Uuid::new_v4().simple()),
            "operator",
            None,
        )
        .await
        .unwrap();
    let store = db.entity_deletion_store();
    store
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    store
        .claim(request.id, "worker", Duration::from_secs(300))
        .await
        .unwrap()
        .unwrap()
}

fn user() -> FenceIdentity {
    FenceIdentity::user(nostr::Keys::generate().public_key())
}

async fn assert_plain_schema(pool: &PgPool) {
    let guards: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger WHERE tgrelid = 'user_deletion_fences'::regclass",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(guards, 0);
    let fks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint
         WHERE conrelid = 'user_deletion_fences'::regclass AND contype = 'f'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(fks, 0);
    crate::Db::from_pool(pool.clone())
        .validate_deletion_catalog()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desired_schema_is_global_and_has_no_cascades_or_triggers() {
    assert_plain_schema(&pool().await).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_is_global_and_has_no_cascades_or_triggers() {
    let pool = pool().await;
    crate::Db::from_pool(pool.clone()).migrate().await.unwrap();
    assert_plain_schema(&pool).await;
    let registered: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM _operator_global_tables WHERE table_name = 'user_deletion_fences')",
    ).fetch_one(&pool).await.unwrap();
    assert!(registered);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn fences_are_atomic_idempotent_scoped_and_preserve_provenance() {
    let pool = pool().await;
    let lease = request(&pool).await;
    let human = user();
    let agent_key = nostr::Keys::generate().public_key();
    let a = FenceIdentity::agent(CommunityId::from_uuid(Uuid::new_v4()), agent_key);
    let b = FenceIdentity::agent(CommunityId::from_uuid(Uuid::new_v4()), agent_key);
    let mut tx = pool.begin().await.unwrap();
    lock_request(&mut tx, &lease).await.unwrap();
    lock_exclusive(&mut tx, &[a, human]).await.unwrap();
    insert(&mut tx, human, &lease, None).await.unwrap();
    insert(&mut tx, a, &lease, Some(human.pubkey))
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(find(&mut tx, human).await.unwrap().is_none());
    assert!(find(&mut tx, a).await.unwrap().is_none());
    tx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    lock_request(&mut tx, &lease).await.unwrap();
    lock_exclusive(&mut tx, &[a, human]).await.unwrap();
    let first = insert(&mut tx, human, &lease, None).await.unwrap();
    let scoped = insert(&mut tx, a, &lease, Some(human.pubkey))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // A new connection/transaction reads durable state, not an in-memory cache.
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(insert(&mut tx, human, &lease, None).await.unwrap(), first);
    assert_eq!(
        insert(&mut tx, a, &lease, Some(human.pubkey))
            .await
            .unwrap(),
        scoped
    );
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_deletion_fences")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        count, 2,
        "retries must not create duplicate global or scoped rows"
    );
    assert!(find(&mut tx, b).await.unwrap().is_none());
    assert!(find(&mut tx, FenceIdentity::user(agent_key))
        .await
        .unwrap()
        .is_none());
    tx.commit().await.unwrap();
    let other_lease = request(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    assert!(insert(&mut tx, human, &other_lease, None).await.is_err());
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(insert(&mut tx, a, &lease, Some(user().pubkey))
        .await
        .is_err());
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(find(&mut tx, human).await.unwrap(), Some(first));
    assert_eq!(find(&mut tx, a).await.unwrap(), Some(scoped));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rejects_missing_requests_and_invalid_owner_shape() {
    let pool = pool().await;
    let lease = request(&pool).await;
    let human = user();
    let agent = FenceIdentity::agent(CommunityId::from_uuid(Uuid::new_v4()), user().pubkey);
    let mut tx = pool.begin().await.unwrap();
    let missing = EntityLease {
        request_id: Uuid::new_v4(),
        ..lease.clone()
    };
    assert!(insert(&mut tx, human, &missing, None).await.is_err());
    assert!(insert(&mut tx, human, &lease, Some(user().pubkey))
        .await
        .is_err());
    assert!(insert(&mut tx, agent, &lease, None).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_deletion_fences")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn identity_locks_share_conflict_and_release_at_transaction_end() {
    let pool = pool().await;
    let human = user();
    let mut first = pool.begin().await.unwrap();
    lock_shared(&mut first, &[human]).await.unwrap();
    let mut second = pool.begin().await.unwrap();
    lock_shared(&mut second, &[human]).await.unwrap();
    second.commit().await.unwrap();
    let mut second = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(&mut *second)
        .await
        .unwrap();
    let error = lock_exclusive(&mut second, &[human]).await.unwrap_err();
    assert!(error.to_string().contains("lock timeout"), "{error}");
    second.rollback().await.unwrap();
    first.commit().await.unwrap();

    let mut gate = pool.begin().await.unwrap();
    lock_exclusive(&mut gate, &[human]).await.unwrap();
    let mut normal = pool.begin().await.unwrap();
    // An unrelated identity is not blocked.
    lock_shared(&mut normal, &[user()]).await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(&mut *normal)
        .await
        .unwrap();
    assert!(lock_shared(&mut normal, &[human]).await.is_err());
    normal.rollback().await.unwrap();
    gate.rollback().await.unwrap();
    let mut normal = pool.begin().await.unwrap();
    lock_shared(&mut normal, &[human]).await.unwrap();
    let timeout: String = sqlx::query_scalar("SHOW lock_timeout")
        .fetch_one(&mut *normal)
        .await
        .unwrap();
    assert_eq!(timeout, "0");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn read_waits_for_gate_then_observes_committed_restriction() {
    let pool = pool().await;
    let lease = request(&pool).await;
    let human = user();
    let mut gate = pool.begin().await.unwrap();
    let saved = insert(&mut gate, human, &lease, None).await.unwrap();
    let mut normal = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *normal)
        .await
        .unwrap();
    let reader = tokio::spawn(async move { find(&mut normal, human).await.unwrap() });
    // Observe the actual lock wait before committing, rather than assuming a
    // sleep was long enough for the competing transaction to reach the guard.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid = $1 AND locktype = 'advisory' AND NOT granted)",
            ).bind(pid).fetch_one(&pool).await.unwrap();
            if waiting { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    gate.commit().await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), reader)
            .await
            .unwrap()
            .unwrap(),
        Some(saved)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn batch_order_is_stable_and_agent_locks_are_community_scoped() {
    let pool = pool().await;
    let human = user();
    let a = FenceIdentity::agent(CommunityId::from_uuid(Uuid::new_v4()), user().pubkey);
    let b = FenceIdentity::agent(CommunityId::from_uuid(Uuid::new_v4()), a.pubkey);
    let mut first = pool.begin().await.unwrap();
    lock_exclusive(&mut first, &[a]).await.unwrap();
    let mut second = pool.begin().await.unwrap();
    lock_exclusive(&mut second, &[b]).await.unwrap();
    second.commit().await.unwrap();
    first.commit().await.unwrap();
    let run = |identities: Vec<FenceIdentity>| {
        let pool = pool.clone();
        async move {
            let mut tx = pool.begin().await.unwrap();
            lock_exclusive(&mut tx, &identities).await.unwrap();
            tx.commit().await.unwrap();
        }
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(run(vec![a, human, b, human]), run(vec![b, a, human]));
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rejects_stale_snapshot_isolation_and_oversized_lock_sets() {
    let pool = pool().await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(find(&mut tx, user()).await.is_err());
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(lock_exclusive(&mut tx, &vec![user(); 4097]).await.is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn insert_requires_approved_inventory_and_current_unexpired_worker() {
    let pool = pool().await;
    let db = crate::Db::from_pool(pool.clone());
    let store = db.entity_deletion_store();
    let submitted = db
        .user_deletion_store()
        .submit("demo:unapproved", "operator", None)
        .await
        .unwrap();
    assert!(store
        .claim(submitted.id, "worker", Duration::from_secs(300))
        .await
        .unwrap()
        .is_none());
    let unapproved = EntityLease {
        request_id: submitted.id,
        owner: "worker".into(),
        generation: 0,
    };
    let lease = request(&pool).await;
    let wrong_worker = EntityLease {
        owner: "other-worker".into(),
        ..lease.clone()
    };
    let wrong_generation = EntityLease {
        generation: lease.generation + 1,
        ..lease.clone()
    };
    let identity = user();
    for rejected in [&unapproved, &wrong_worker, &wrong_generation] {
        let mut tx = pool.begin().await.unwrap();
        assert!(matches!(
            insert(&mut tx, identity, rejected, None).await,
            Err(DbError::AccessDenied(_))
        ));
        tx.rollback().await.unwrap();
    }
    // Expire deterministically instead of racing a short wall-clock timeout.
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(lease.request_id).execute(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        insert(&mut tx, identity, &lease, None).await,
        Err(DbError::AccessDenied(_))
    ));
    tx.rollback().await.unwrap();
    let successor = store
        .claim(lease.request_id, "successor", Duration::from_secs(300))
        .await
        .unwrap()
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        insert(&mut tx, identity, &lease, None).await,
        Err(DbError::AccessDenied(_))
    ));
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(find(&mut tx, identity).await.unwrap().is_none());
    tx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let saved = insert(&mut tx, identity, &successor, None).await.unwrap();
    tx.commit().await.unwrap();
    // Exercise the insertion guard independently of claim-time validation.
    sqlx::query("DELETE FROM entity_deletion_approvals WHERE request_id = $1")
        .bind(successor.request_id)
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        insert(&mut tx, user(), &successor, None).await,
        Err(DbError::DeletionSafety(_))
    ));
    tx.rollback().await.unwrap();
    let rows: Vec<UserDeletionFence> = sqlx::query_as(
        "SELECT id, pubkey, scope_community_id, request_id, source_owner_pubkey, blocked_at FROM user_deletion_fences"
    ).fetch_all(&pool).await.unwrap();
    assert_eq!(rows, vec![saved]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn competing_inserts_wait_then_preserve_or_reject_original_provenance() {
    let pool = pool().await;
    let lease = request(&pool).await;
    let other_lease = request(&pool).await;
    for (scoped, conflict) in [(false, false), (false, true), (true, false), (true, true)] {
        let owner = user().pubkey;
        let identity = if scoped {
            FenceIdentity::agent(CommunityId::from_uuid(Uuid::new_v4()), user().pubkey)
        } else {
            user()
        };
        let source_owner = scoped.then_some(owner);
        let mut first = pool.begin().await.unwrap();
        let saved = insert(&mut first, identity, &lease, source_owner)
            .await
            .unwrap();
        let next_lease = if conflict && !scoped {
            other_lease.clone()
        } else {
            lease.clone()
        };
        let next_owner = if conflict && scoped {
            Some(user().pubkey)
        } else {
            source_owner
        };
        let mut second = pool.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *second)
            .await
            .unwrap();
        let writer = tokio::spawn(async move {
            let result = insert(&mut second, identity, &next_lease, next_owner).await;
            if result.is_ok() {
                second.commit().await.unwrap();
            } else {
                second.rollback().await.unwrap();
            }
            result
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid = $1 AND NOT granted)",
                )
                .bind(pid)
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
        .unwrap();
        first.commit().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .unwrap()
            .unwrap();
        if conflict {
            assert!(
                matches!(result, Err(DbError::DeletionSafety(ref message)) if message.contains("different provenance"))
            );
        } else {
            assert_eq!(result.unwrap(), saved);
        }
        let mut tx = pool.begin().await.unwrap();
        assert_eq!(find(&mut tx, identity).await.unwrap(), Some(saved));
        tx.commit().await.unwrap();
    }
}
