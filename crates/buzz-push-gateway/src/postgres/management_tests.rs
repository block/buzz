mod management_postgres_tests {
    use super::*;
    fn assertion(previous: u32, next: u32, expected: Option<i64>, now: i64) -> AssertionCounter {
        AssertionCounter {
            request_expires_at: now + 300,
            previous,
            next,
            expected_generation: expected,
            now,
        }
    }
    fn delegation(hash: [u8; 32], relay: &str, generation: i64, now: i64) -> Delegation {
        Delegation {
            grant_hash: hash,
            id: Uuid::from_bytes(hash[..16].try_into().unwrap()),
            installation_id: Uuid::from_u128(1),
            relay_pubkey: relay.to_owned(),
            endpoint_epoch: 1,
            generation,
            not_before: now,
            expires_at: now + 1_000,
            revoked: false,
        }
    }
    async fn deliver(
        store: &PostgresAuthorityStore,
        d: &Delegation,
        event: u8,
        request: Uuid,
        cap: i64,
        now: i64,
    ) -> Result<DeliveryPermit, AuthorityError> {
        store
            .authorize_delivery(
                d.grant_hash,
                &d.relay_pubkey,
                &hex::encode([event; 32]),
                request,
                now + 100,
                60,
                cap,
                now,
            )
            .await
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn concurrent_renewal_revocation_and_delivery_preserve_authority_fences() {
        let (pool, schema) = full_schema(4).await;
        install_authority(&pool).await;
        let store = PostgresAuthorityStore::new(pool.clone());
        let now = Utc::now().timestamp();
        let d = delegation([1; 32], RELAY_HEX, 2, now);
        let (renew, revoke) = tokio::join!(
            store.upsert_delegation(d, assertion(0, 1, Some(1), now)),
            store.revoke_delegation(Uuid::from_u128(1), RELAY_HEX, 1, assertion(0, 2, None, now))
        );
        assert_eq!(
            usize::from(renew.is_ok()) + usize::from(revoke.is_ok()),
            1,
            "counter and permission mutation commit together"
        );
        let counter = store
            .installation(Uuid::from_u128(1), now)
            .await
            .unwrap()
            .assertion_counter;
        let generation: i64 = sqlx::query_scalar("SELECT generation FROM push_gateway_delegations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(generation, 2);
        assert_eq!(counter, if renew.is_ok() { 1 } else { 2 });
        // Regardless of who won, no old delivery can pass the version fence.
        assert!(
            admit(&store, &"22".repeat(32), Uuid::new_v4())
                .await
                .is_err()
        );
        if renew.is_ok() {
            let d = delegation([1; 32], RELAY_HEX, 2, now);
            let (delivery, revocation) = tokio::join!(
                deliver(&store, &d, 3, Uuid::new_v4(), 40, now),
                store.revoke_delegation(
                    d.installation_id,
                    RELAY_HEX,
                    2,
                    assertion(1, 3, None, now)
                )
            );
            revocation.unwrap();
            // A previously admitted send may finish; revocation gates new admissions.
            drop(delivery);
            assert!(
                deliver(&store, &d, 4, Uuid::new_v4(), 40, now)
                    .await
                    .is_err()
            );
        }
        pool.close().await;
        drop_schema(&schema).await;
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn maximum_generation_retains_revocation_capacity() {
        let (pool, schema) = full_schema(2).await;
        install_authority(&pool).await;
        let store = PostgresAuthorityStore::new(pool.clone());
        let now = Utc::now().timestamp();
        let d = delegation([17; 32], RELAY_HEX, i64::MAX, now);
        store
            .upsert_delegation(d.clone(), assertion(0, 1, Some(1), now))
            .await
            .unwrap();
        deliver(&store, &d, 18, Uuid::new_v4(), 40, now)
            .await
            .unwrap();
        for (previous, next) in [(1, 2), (2, 3)] {
            store
                .revoke_delegation(
                    d.installation_id,
                    RELAY_HEX,
                    i64::MAX,
                    assertion(previous, next, None, now),
                )
                .await
                .unwrap();
        }
        assert!(
            deliver(&store, &d, 19, Uuid::new_v4(), 40, now)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .upsert_delegation(d, assertion(3, 4, Some(i64::MAX), now))
                .await,
            Err(AuthorityError::StaleGeneration(i64::MAX))
        );
        pool.close().await;
        drop_schema(&schema).await;
    }
}
