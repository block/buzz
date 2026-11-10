use super::postgres_tests::{activate, enqueue_one, make_community, quiesce_test_community};
use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn empty_communities_do_not_require_wake_claims() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("isolated database");
    sqlx::query(
        "INSERT INTO communities (id, host) \
         SELECT gen_random_uuid(), 'empty-' || n || '.example' FROM generate_series(1, 1000) n",
    )
    .execute(&pool)
    .await
    .expect("seed empty communities");

    assert!(due_wake_communities(&pool, None)
        .await
        .expect("discover ready work")
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn discovery_preserves_claim_eligibility_and_recovers_expired_workers() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let author = [21; 32];
    let mut expected = Vec::new();
    for case in [
        "pending",
        "expired_worker",
        "live_worker",
        "future_retry",
        "expired_wake",
        "inactive_lease",
        "disabled_endpoint",
        "expired_lease",
        "old_generation",
        "old_endpoint",
        "deleted_event",
        "missing_event",
        "archived",
        "quiescing",
    ] {
        let community = make_community(&pool).await;
        activate(&pool, community, &author, "install", &[22; 32], 1).await;
        let id = enqueue_one(&pool, community, &author, &[23; 32], 1).await;
        let update = match case {
            "pending" => {
                expected.push(community);
                continue;
            }
            "expired_worker" => {
                expected.push(community);
                "UPDATE push_wake_outbox SET state = 'sending', lease_until = now() - interval '1 second' WHERE community_id = $1"
            }
            "live_worker" => "UPDATE push_wake_outbox SET state = 'sending', lease_until = now() + interval '1 hour' WHERE community_id = $1",
            "future_retry" => "UPDATE push_wake_outbox SET next_attempt_at = now() + interval '1 hour' WHERE community_id = $1",
            "expired_wake" => "UPDATE push_wake_outbox SET expires_at = 1 WHERE community_id = $1",
            "inactive_lease" => "UPDATE push_leases SET active = false, endpoint_hash = NULL, endpoint_grant = NULL, max_class = NULL, subscriptions = NULL WHERE community_id = $1",
            "disabled_endpoint" => "UPDATE push_leases SET endpoint_enabled = false WHERE community_id = $1",
            "expired_lease" => "UPDATE push_leases SET expires_at = 1 WHERE community_id = $1",
            "old_generation" => "UPDATE push_wake_outbox SET lease_generation = 2 WHERE community_id = $1",
            "old_endpoint" => "UPDATE push_wake_outbox SET endpoint_hash = decode(repeat('ff', 32), 'hex') WHERE community_id = $1",
            "deleted_event" => "UPDATE events SET deleted_at = now() WHERE community_id = $1",
            "missing_event" => "DELETE FROM events WHERE community_id = $1",
            "archived" => "UPDATE communities SET archived_at = now() WHERE id = $1",
            "quiescing" => {
                quiesce_test_community(&pool, community).await;
                continue;
            },
            _ => unreachable!(),
        };
        sqlx::query(update)
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        if case == "expired_worker" {
            // Discovery does not claim: the original request ID is still recovered
            // by the unchanged claim API, fenced with a fresh claim ID.
            let discovered = due_wake_communities(&pool, None).await.unwrap();
            assert!(discovered.contains(&community));
            let wake = claim_due_wakes(
                &pool,
                community,
                16,
                Utc::now() + chrono::Duration::seconds(30),
            )
            .await
            .unwrap()
            .pop()
            .unwrap();
            assert_eq!(wake.id, id);
            assert!(retry_wake(
                &pool,
                community,
                id,
                wake.claim_id,
                Utc::now() - chrono::Duration::seconds(1)
            )
            .await
            .unwrap());
        }
    }
    expected.sort_by_key(|community| *community.as_uuid());
    assert_eq!(due_wake_communities(&pool, None).await.unwrap(), expected);

    // A lifecycle transition after discovery is still rejected at admission.
    let community = expected[0];
    quiesce_test_community(&pool, community).await;
    assert!(matches!(
        claim_due_wakes(
            &pool,
            community,
            16,
            Utc::now() + chrono::Duration::seconds(30)
        )
        .await,
        Err(crate::DbError::AccessDenied(_))
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn discovery_pages_past_busy_communities_without_claiming_them() {
    // One connection also proves discovery releases its checkout before claims.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let mut communities = Vec::new();
    for _ in 0..70 {
        let community = make_community(&pool).await;
        activate(&pool, community, &[24; 32], "install", &[25; 32], 1).await;
        enqueue_one(&pool, community, &[24; 32], &[26; 32], 1).await;
        communities.push(community);
    }
    communities.sort_by_key(|community| *community.as_uuid());
    let first = due_wake_communities(&pool, None).await.unwrap();
    assert_eq!(first, communities[..64]);
    // Leave the first page completely busy: later communities still get a turn.
    let second = due_wake_communities(&pool, first.last().copied())
        .await
        .unwrap();
    assert_eq!(second, communities[64..]);
    assert!(due_wake_communities(&pool, second.last().copied())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(due_wake_communities(&pool, None).await.unwrap(), first);
    let wake = claim_due_wakes(
        &pool,
        second[0],
        16,
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await
    .unwrap();
    assert_eq!(wake.len(), 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invalid_backlog_cannot_hide_ready_communities() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    sqlx::query("INSERT INTO communities (host) SELECT 'empty-' || n || '.example' FROM generate_series(1,1000) n")
        .execute(&pool).await.unwrap();
    let mut expected = Vec::new();
    for n in 0..136 {
        // Fixed ordering puts more than one page of invalid work before live work.
        let community = CommunityId::from_uuid(Uuid::from_u128(n + 1));
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community.as_uuid())
            .bind(format!("ready-{n}.example"))
            .execute(&pool)
            .await
            .unwrap();
        activate(&pool, community, &[31; 32], "install", &[32; 32], 1).await;
        enqueue_one(&pool, community, &[31; 32], &[33; 32], 1).await;
        if n < 128 {
            sqlx::query("UPDATE push_wake_outbox SET lease_generation = 2 WHERE community_id = $1")
                .bind(community.as_uuid())
                .execute(&pool)
                .await
                .unwrap();
        } else {
            expected.push(community);
        }
    }
    // Retained terminal jobs must not turn this into an all-history scan. These
    // rows have no source event, which is legal after event retention/deletion.
    sqlx::query(
        r#"
        INSERT INTO push_wake_outbox (community_id, author, installation_id, lease_generation,
            endpoint_hash, event_id, class, expires_at, state)
        SELECT community_id, author, installation_id, generation, endpoint_hash,
               decode(lpad(to_hex(n),64,'0'),'hex'), 'default', expires_at, 'delivered'
        FROM push_leases CROSS JOIN generate_series(1,200) n
    "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        "ANALYZE communities; ANALYZE push_wake_outbox; ANALYZE push_leases; ANALYZE events;",
    )
    .execute(&pool)
    .await
    .unwrap();
    let started = std::time::Instant::now();
    for _ in 0..10 {
        assert_eq!(due_wake_communities(&pool, None).await.unwrap(), expected);
    }
    eprintln!("10 discoveries with 1,136 communities, 128 invalid jobs, 27,200 terminal jobs, 8 ready jobs: {:?}", started.elapsed());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_discovery_preserves_exclusive_claims_and_recovery_fencing() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let community = make_community(&pool).await;
    activate(&pool, community, &[41; 32], "install", &[42; 32], 1).await;
    for n in 0..32 {
        enqueue_one(&pool, community, &[41; 32], &[n; 32], 1).await;
    }
    // Discovery is deliberately not exclusive: both workers can see the same
    // candidate, while the existing claim transaction owns exclusivity.
    let (a, b) = tokio::join!(
        due_wake_communities(&pool, None),
        due_wake_communities(&pool, None)
    );
    assert_eq!(a.unwrap(), vec![community]);
    assert_eq!(b.unwrap(), vec![community]);
    let until = Utc::now() + chrono::Duration::seconds(30);
    let (a, b) = tokio::join!(
        claim_due_wakes(&pool, community, 16, until),
        claim_due_wakes(&pool, community, 16, until),
    );
    let mut claims = a.unwrap();
    claims.extend(b.unwrap());
    assert_eq!(claims.len(), 32);
    assert_eq!(
        claims
            .iter()
            .map(|wake| wake.id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        32
    );
    assert!(due_wake_communities(&pool, None).await.unwrap().is_empty());

    let old = &claims[0];
    sqlx::query("UPDATE push_wake_outbox SET lease_until = now() - interval '1 second' WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid()).bind(old.id).execute(&pool).await.unwrap();
    assert_eq!(
        due_wake_communities(&pool, None).await.unwrap(),
        vec![community]
    );
    let recovered = claim_due_wakes(&pool, community, 16, until).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].id, old.id,
        "gateway idempotency key survives recovery"
    );
    assert_ne!(recovered[0].claim_id, old.claim_id);
    assert!(matches!(
        revalidate_wake_for_send(&pool, community, old.id, old.claim_id)
            .await
            .unwrap(),
        RevalidateWakeOutcome::Suppressed
    ));
    assert!(!complete_wake(&pool, community, old.id, old.claim_id)
        .await
        .unwrap());
    assert!(
        !retry_wake(&pool, community, old.id, old.claim_id, Utc::now())
            .await
            .unwrap()
    );
    assert!(!fail_wake(&pool, community, old.id, old.claim_id)
        .await
        .unwrap());
    assert!(
        complete_wake(&pool, community, old.id, recovered[0].claim_id)
            .await
            .unwrap()
    );
}
