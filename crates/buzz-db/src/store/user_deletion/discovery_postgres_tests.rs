use super::*;
use nostr::ToBech32;
use serde_json::json;
use uuid::Uuid;

const PUBKEY: &str = "5e4083a7a793d02575876eaf47932b4835999f5af2653d26250a909b450da7c9";

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owner_preview_is_available_but_submission_rolls_back_until_scope_is_supported() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, 'owner.inventory.example')")
        .bind(community)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'owner')")
        .bind(community)
        .bind(PUBKEY)
        .execute(&pool)
        .await
        .unwrap();
    let preview = store.generate_inventory(PUBKEY).await.unwrap();
    assert_eq!(preview["communities"][0]["relationship"], "owner");
    let error = store.submit(PUBKEY, "operator", None).await.unwrap_err();
    assert!(error
        .to_string()
        .contains("owned-community execution is not supported yet"));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entity_deletion_requests WHERE target = $1")
            .bind(PUBKEY)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);

    // A refused submission must not reserve the target or retain stale inventory.
    sqlx::query("UPDATE relay_members SET role = 'member' WHERE community_id = $1 AND pubkey = $2")
        .bind(community)
        .bind(PUBKEY)
        .execute(&pool)
        .await
        .unwrap();
    let saved = store.submit(PUBKEY, "operator", None).await.unwrap();
    assert_eq!(saved.status, "inventoried");
    assert_eq!(
        saved.inventory_manifest.as_ref().unwrap()["communities"][0]["relationship"],
        "participant"
    );
    store
        .approve(
            saved.id,
            "approver",
            saved.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_memberships_discover_participants_deduplicate_and_preserve_owners() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let mut expected = Vec::new();
    for (relay_role, removed, unrelated) in [
        (None, false, false),
        (Some("owner"), false, false),
        (Some("admin"), false, false),
        (Some("member"), false, false),
        (None, true, false),
        (None, false, true),
    ] {
        let community = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("{community}.inventory.example"))
            .execute(&pool)
            .await
            .unwrap();
        if let Some(role) = relay_role {
            sqlx::query(
                "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, $3)",
            )
            .bind(community)
            .bind(PUBKEY)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
        }
        let key = if unrelated {
            nostr::Keys::generate().public_key().to_hex()
        } else {
            PUBKEY.to_owned()
        };
        // Reuse channel IDs across communities to exercise community-scoped identity.
        // Two hidden channel memberships still count as one participating community.
        for channel in [Uuid::from_u128(1), Uuid::from_u128(2)] {
            sqlx::query("INSERT INTO channels (community_id, id, name, created_by) VALUES ($1, $2, 'inventory-test', decode($3, 'hex'))")
                .bind(community).bind(channel).bind(&key)
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey, removed_at, hidden_at) VALUES ($1, $2, decode($3, 'hex'), CASE WHEN $4 THEN now() END, now())")
                .bind(community).bind(channel).bind(&key).bind(removed)
                .execute(&pool).await.unwrap();
        }
        if !removed && !unrelated {
            expected.push((community, relay_role));
        }
    }
    expected.sort_by_key(|(community, _)| *community);
    let store = Db::from_pool(pool).user_deletion_store();
    let inventory = store.generate_inventory(PUBKEY).await.unwrap();
    let communities = inventory["communities"].as_array().unwrap();
    assert_eq!(communities.len(), expected.len());
    for (actual, (community, role)) in communities.iter().zip(expected) {
        assert_eq!(actual["community_id"], community.to_string());
        assert_eq!(
            actual["relationship"],
            if role == Some("owner") {
                "owner"
            } else {
                "participant"
            }
        );
        assert_eq!(actual.as_object().unwrap().len(), 3);
    }
    let npub = nostr::PublicKey::parse(PUBKEY)
        .unwrap()
        .to_bech32()
        .unwrap();
    assert_eq!(store.generate_inventory(&npub).await.unwrap(), inventory);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn current_inventory_includes_all_lifecycle_states_and_excludes_removed_memberships() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    let mut expected = Vec::new();
    for (role, state, archived) in [
        ("owner", "active", false),
        ("admin", "active", false),
        ("member", "active", false),
        ("member", "active", true),
        ("owner", "quiescing", false),
        ("member", "fenced", true),
        ("member", "tombstone", false),
    ] {
        let id = Uuid::new_v4();
        let host = format!("{id}.inventory.example");
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(PUBKEY)
            .bind(role)
            .execute(&mut *tx)
            .await
            .unwrap();
        // Lifecycle fixture changes use the existing community executor fence.
        sqlx::query("SELECT set_config('buzz.deletion_executor_community', $1, true), set_config('buzz.deletion_fence_generation', '0', true)")
            .bind(id.to_string()).execute(&mut *tx).await.unwrap();
        sqlx::query("UPDATE communities SET deletion_state = $2, archived_at = CASE WHEN $3 THEN '2026-01-01'::timestamptz END, deleted_at = CASE WHEN $2 = 'tombstone' THEN '2026-01-01'::timestamptz END WHERE id = $1")
            .bind(id).bind(state).bind(archived).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        expected.push(json!({
            "community_id": id, "label": host,
            "relationship": if role == "owner" { "owner" } else { "participant" },
        }));
    }
    let former = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(former)
        .bind(format!("{former}.former.example"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(former)
        .bind(PUBKEY)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(former)
        .bind(PUBKEY)
        .execute(&pool)
        .await
        .unwrap();
    // An unrelated identity in this community must not count as our membership.
    let other = nostr::Keys::generate().public_key().to_hex();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'owner')")
        .bind(former)
        .bind(&other)
        .execute(&pool)
        .await
        .unwrap();

    expected.sort_by(|a, b| a["community_id"].as_str().cmp(&b["community_id"].as_str()));
    let inventory = store.generate_inventory(PUBKEY).await.unwrap();
    assert_eq!(
        inventory,
        json!({
            "communities": expected,
        })
    );
    let npub = nostr::PublicKey::parse(PUBKEY)
        .unwrap()
        .to_bech32()
        .unwrap();
    assert_eq!(store.generate_inventory(&npub).await.unwrap(), inventory);
    assert_eq!(
        store
            .generate_inventory(&nostr::Keys::generate().public_key().to_hex())
            .await
            .unwrap()["communities"],
        json!([])
    );
    pool.close().await;
    assert!(
        store.generate_inventory(PUBKEY).await.is_err(),
        "a failed lookup must not become an empty inventory"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn oversized_inventory_fails_instead_of_truncating() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let pubkey = nostr::Keys::generate().public_key().to_hex();
    sqlx::query("WITH inserted AS (INSERT INTO communities (host) SELECT gen_random_uuid()::text || '.inventory.example' FROM generate_series(1, 1001) RETURNING id) INSERT INTO relay_members (community_id, pubkey, role) SELECT id, $1, 'member' FROM inserted")
        .bind(&pubkey).execute(&pool).await.unwrap();
    let result = Db::from_pool(pool)
        .user_deletion_store()
        .generate_inventory(&pubkey)
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("no partial inventory returned"));
}
