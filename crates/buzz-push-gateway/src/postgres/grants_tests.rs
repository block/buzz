mod grants_postgres_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn random_grants_require_full_hash_signer_validity_and_current_permission() {
        let (pool, schema) = full_schema(4).await;
        install_authority(&pool).await;
        let store = PostgresAuthorityStore::new(pool.clone());
        let now = Utc::now().timestamp();
        let (token, hash) = crate::grant::issue().unwrap();
        let d = Delegation {
            id: crate::grant::id(&hash),
            grant_hash: hash,
            installation_id: Uuid::from_u128(1),
            relay_pubkey: RELAY_HEX.into(),
            endpoint_epoch: 1,
            generation: 2,
            not_before: now,
            expires_at: now + 600,
            revoked: false,
        };
        store.upsert_delegation(d.clone()).await.unwrap();
        let stored: Vec<u8> = sqlx::query_scalar("SELECT grant_hash FROM push_gateway_delegations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, crate::grant::hash(&token).unwrap());
        assert_ne!(stored, token.as_bytes());
        assert!(
            admit(&store, &"22".repeat(32), Uuid::new_v4())
                .await
                .is_err()
        );
        let mut tampered = hash;
        tampered[31] ^= 1;
        for (presented, relay, when) in [
            (tampered, RELAY_HEX, now),
            (hash, &"66".repeat(32), now),
            (hash, RELAY_HEX, now + 601),
        ] {
            assert!(
                store
                    .authorize_delivery(
                        presented,
                        relay,
                        &"33".repeat(32),
                        Uuid::new_v4(),
                        when + 60,
                        60,
                        10,
                        when
                    )
                    .await
                    .is_err()
            );
        }
        // Rows from before the additive migration have no hash; they cannot authorize.
        sqlx::query("UPDATE push_gateway_delegations SET grant_hash=NULL")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            store
                .authorize_delivery(
                    hash,
                    RELAY_HEX,
                    &"77".repeat(32),
                    Uuid::new_v4(),
                    now + 60,
                    60,
                    10,
                    now
                )
                .await
                .is_err()
        );
        sqlx::query("UPDATE push_gateway_delegations SET grant_hash=$1")
            .bind(hash.as_slice())
            .execute(&pool)
            .await
            .unwrap();
        store
            .authorize_delivery(
                hash,
                RELAY_HEX,
                &"44".repeat(32),
                Uuid::new_v4(),
                now + 60,
                60,
                10,
                now,
            )
            .await
            .unwrap();
        store
            .revoke_delegation(d.installation_id, RELAY_HEX, 2)
            .await
            .unwrap();
        assert!(
            store
                .authorize_delivery(
                    hash,
                    RELAY_HEX,
                    &"55".repeat(32),
                    Uuid::new_v4(),
                    now + 60,
                    60,
                    10,
                    now
                )
                .await
                .is_err()
        );
        pool.close().await;
        drop_schema(&schema).await;
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn delegation_catalog_matches_desired_state_after_expansion() {
        let (pool, schema) = full_schema(1).await;
        let migrated:Vec<(String,String,bool)>=sqlx::query_as("SELECT attname::text,format_type(atttypid,atttypmod),attnotnull FROM pg_attribute WHERE attrelid='push_gateway_delegations'::regclass AND attnum>0 AND NOT attisdropped ORDER BY attname").fetch_all(&pool).await.unwrap();
        sqlx::raw_sql("ALTER TABLE push_gateway_delegations RENAME TO migrated_delegations")
            .execute(&pool)
            .await
            .unwrap();
        // Existing index names follow the renamed table, so compare columns after
        // creating the declarative table under its own name.
        let table = include_str!("../../../../schema/tables/public/push_gateway_delegations.sql");
        sqlx::raw_sql(table.split("CREATE INDEX").next().unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let desired:Vec<(String,String,bool)>=sqlx::query_as("SELECT attname::text,format_type(atttypid,atttypmod),attnotnull FROM pg_attribute WHERE attrelid='push_gateway_delegations'::regclass AND attnum>0 AND NOT attisdropped ORDER BY attname").fetch_all(&pool).await.unwrap();
        assert_eq!(migrated, desired);
        pool.close().await;
        drop_schema(&schema).await;
    }
}
