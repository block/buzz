use super::*;
use serde_json::{json, Value};
use uuid::Uuid;

async fn community(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(id)
        .bind(format!("{id}.example.org"))
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn event(pool: &PgPool, community: Uuid, author: &str, kind: i32, tags: Value) {
    sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig,d_tag)
        VALUES($1,decode(replace(gen_random_uuid()::text,'-','') || replace(gen_random_uuid()::text,'-',''),'hex'),
        decode($2,'hex'),now(),$3,$4,'residue',decode(repeat('00',64),'hex'),$4->0->>1)")
        .bind(community).bind(author).bind(kind).bind(tags).execute(pool).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn shared_inventory_discovers_residue_and_scoped_agents_without_unrelated_communities() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let store = db.user_deletion_store();
    let human = nostr::Keys::generate().public_key().to_hex();
    let agent = nostr::Keys::generate().public_key().to_hex();
    let other = nostr::Keys::generate().public_key().to_hex();
    let relay = nostr::Keys::generate().public_key().to_hex();
    let mut expected = Vec::new();
    // Each community has only one source of attribution: no users/membership
    // row can accidentally mask a missing historical-data predicate.
    for (sql, role) in [
        ("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'admin')", "participant"),
        ("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')", "participant"),
        ("INSERT INTO channel_members(community_id,channel_id,pubkey) VALUES($1,gen_random_uuid(),decode($2,'hex'))", "participant"),
        ("INSERT INTO channel_members(community_id,channel_id,pubkey,removed_at) VALUES($1,gen_random_uuid(),decode($2,'hex'),now())", "past_participant"),
        ("INSERT INTO api_tokens(community_id,token_hash,owner_pubkey,name,scopes,revoked_at) VALUES($1,decode($2,'hex'),decode($2,'hex'),'old','[]',now())", "past_participant"),
        ("INSERT INTO personal_read_accounts(community_id,actor) VALUES($1,decode($2,'hex'))", "past_participant"),
        ("INSERT INTO nip_fi_profile_heads(community_id,pubkey,issuer,issued_at,event) VALUES($1,$2,'example.org',1,'{}')", "past_participant"),
        ("INSERT INTO media_uploaders(community_id,pubkey,sha256) VALUES($1,decode($2,'hex'),repeat('01',32))", "past_participant"),
        ("INSERT INTO pubkey_allowlist(community_id,pubkey,added_by) VALUES($1,decode(repeat('02',32),'hex'),decode($2,'hex'))", "past_participant"),
    ] {
        let id = community(&pool).await;
        sqlx::query(sql).bind(id).bind(&human).execute(&pool).await.unwrap();
        expected.push((id, role));
    }
    let archived = community(&pool).await;
    sqlx::query("INSERT INTO archived_identities(community_id,pubkey,actor,consent_path,request_event_id) VALUES($1,$2,$3,'admin','archive-request')")
        .bind(archived).bind(&other).bind(&human).execute(&pool).await.unwrap();
    expected.push((archived, "past_participant"));
    let owned = community(&pool).await;
    sqlx::query(
        "INSERT INTO users(community_id,pubkey,agent_owner_pubkey,deactivated_at)
        VALUES($1,decode($2,'hex'),decode($3,'hex'),now())",
    )
    .bind(owned)
    .bind(&agent)
    .bind(&human)
    .execute(&pool)
    .await
    .unwrap();
    expected.push((owned, "past_participant"));
    let transferred = community(&pool).await;
    sqlx::query(
        "INSERT INTO users(community_id,pubkey,agent_owner_pubkey)
        VALUES($1,decode($2,'hex'),decode($3,'hex'))",
    )
    .bind(transferred)
    .bind(&agent)
    .bind(&other)
    .execute(&pool)
    .await
    .unwrap();
    event(&pool, transferred, &agent, 9, json!([])).await;
    for (author, kind, tags) in [
        (&human, 9, json!([])),
        (&human, 10030, json!([])),
        (&other, 30382, json!([["d", human], ["p", human]])),
        (&other, 8003, json!([["p", other], ["replaced-by", human]])),
        (&relay, 9, json!([["actor", human], ["p", other]])),
        (&relay, 9, json!([["p", human]])),
        (&relay, 30622, json!([["d", human], ["p", human]])),
    ] {
        let id = community(&pool).await;
        event(&pool, id, author, kind, tags).await;
        expected.push((id, "past_participant"));
    }
    let retained = community(&pool).await;
    event(&pool, retained, &human, 30617, json!([])).await;
    event(&pool, retained, &other, 1, json!([["p", human]])).await;
    event(&pool, retained, &other, 9, json!([["p", human]])).await;
    event(&pool, retained, &other, 9, json!([["actor", human]])).await;
    event(
        &pool,
        retained,
        &other,
        30622,
        json!([["d", human], ["p", human]]),
    )
    .await;
    event(
        &pool,
        retained,
        &relay,
        30622,
        json!([["d", human], ["p", other]]),
    )
    .await;
    community(&pool).await;

    let run = Uuid::new_v4();
    let first = store
        .submit_rehearsal_with_relay(&human, "operator", None, run, Some(&relay))
        .await
        .unwrap();
    let inventory = UserDeletionInventory::read(&first).unwrap();
    expected.sort_by_key(|(id, _)| *id);
    let actual: Vec<_> = inventory
        .communities
        .iter()
        .map(|(id, row)| (*id, row["relationship"].as_str().unwrap()))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(
        first.inventory_manifest.as_ref().unwrap()["scope"],
        "affected_communities"
    );
    let saved = db.entity_deletion_store().inspect(first.id).await.unwrap();
    assert!(saved.approvals.is_empty());
    assert!(saved.stage_history.is_empty());

    // Preview and both submission modes share selection and frozen attribution.
    let preview = store
        .generate_inventory_with_relay(&human, Some(&relay))
        .await
        .unwrap();
    let deletion = store
        .submit_with_relay(&human, "operator", None, Some(&relay))
        .await
        .unwrap();
    assert_eq!(deletion.inventory_manifest.as_ref().unwrap(), &preview);
    let mut rehearsal_inventory = first.inventory_manifest.clone().unwrap();
    for field in ["execution_mode", "run_id"] {
        rehearsal_inventory.as_object_mut().unwrap().remove(field);
    }
    assert_eq!(rehearsal_inventory, preview);
    // No-relay inputs are also shared and exclude relay-only historical attribution.
    let keyless = store
        .submit_rehearsal(&human, "operator", None, Uuid::new_v4())
        .await
        .unwrap();
    let keyless_preview = store.generate_inventory(&human).await.unwrap();
    assert_eq!(
        keyless.inventory_manifest.as_ref().unwrap()["communities"],
        keyless_preview["communities"]
    );
    assert_eq!(
        keyless_preview["communities"].as_array().unwrap().len(),
        expected.len() - 3
    );
    assert!(store
        .generate_inventory_with_relay(&human, Some(&human))
        .await
        .is_err());
    assert!(store
        .submit_with_relay(&human, "operator", None, Some(&human))
        .await
        .is_err());
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,decode($2,'hex'))")
        .bind(retained)
        .bind(&human)
        .execute(&pool)
        .await
        .unwrap();
    let retry = store
        .submit_rehearsal_with_relay(&human, "operator", None, run, Some(&relay))
        .await
        .unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(retry.inventory_manifest, first.inventory_manifest);
    assert_eq!(retry.inventory_digest, first.inventory_digest);
    assert!(store
        .submit_rehearsal_with_relay(&human, "operator", None, run, Some(&other))
        .await
        .is_err());
    let deletion_retry = store
        .submit_with_relay(&human, "operator", None, Some(&relay.to_uppercase()))
        .await
        .unwrap();
    assert_eq!(
        deletion_retry.inventory_manifest,
        deletion.inventory_manifest
    );
    assert_eq!(deletion_retry.inventory_digest, deletion.inventory_digest);
    for relay_input in [None, Some(other.as_str())] {
        assert!(store
            .submit_with_relay(&human, "operator", None, relay_input)
            .await
            .is_err());
    }
    let fresh = store
        .submit_rehearsal_with_relay(&human, "operator", None, Uuid::new_v4(), Some(&relay))
        .await
        .unwrap();
    assert_eq!(
        UserDeletionInventory::read(&fresh)
            .unwrap()
            .communities
            .len(),
        expected.len() + 1
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn shared_inventory_limit_counts_only_matched_communities_and_errors_leave_no_request() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    let human = nostr::Keys::generate().public_key().to_hex();
    sqlx::query("INSERT INTO communities(host) SELECT 'unrelated-' || n || '.example.org' FROM generate_series(1,1001) n")
        .execute(&pool).await.unwrap();
    let empty = store
        .submit_rehearsal(&human, "operator", None, Uuid::new_v4())
        .await
        .unwrap();
    assert!(UserDeletionInventory::read(&empty)
        .unwrap()
        .communities
        .is_empty());
    let id = community(&pool).await;
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,decode($2,'hex'))")
        .bind(id)
        .bind(&human)
        .execute(&pool)
        .await
        .unwrap();
    let small = store
        .submit_rehearsal(&human, "operator", None, Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(
        UserDeletionInventory::read(&small)
            .unwrap()
            .communities
            .len(),
        1
    );

    sqlx::query("INSERT INTO users(community_id,pubkey) SELECT id,decode($1,'hex') FROM communities ON CONFLICT DO NOTHING")
        .bind(&human).execute(&pool).await.unwrap();
    assert!(store
        .submit_rehearsal(&human, "operator", None, Uuid::new_v4())
        .await
        .unwrap_err()
        .to_string()
        .contains("no partial inventory returned"));
    assert!(store.generate_inventory(&human).await.is_err());
    assert!(store.submit(&human, "operator", None).await.is_err());
    sqlx::query("ALTER TABLE api_tokens RENAME TO unavailable_api_tokens")
        .execute(&pool)
        .await
        .unwrap();
    assert!(store
        .submit_rehearsal(&human, "operator", None, Uuid::new_v4())
        .await
        .is_err());
    assert!(store.generate_inventory(&human).await.is_err());
    assert!(store.submit(&human, "operator", None).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM entity_deletion_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}
