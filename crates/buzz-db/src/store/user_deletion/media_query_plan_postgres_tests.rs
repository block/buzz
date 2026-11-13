//! Exercise the production query with sparse targets amid unrelated upload history.
use super::*;
use serde_json::Value;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn media_discovery_and_sharing_use_indexes_without_current_membership() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let community = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(format!("{community}.example.org"))
        .execute(&pool)
        .await
        .unwrap();
    let alice = nostr::Keys::generate().public_key().to_hex();
    let bob = nostr::Keys::generate().public_key().to_hex();
    // Membership was removed; retained attribution is still authoritative.
    sqlx::query("INSERT INTO relay_members(community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(community)
        .bind(&alice)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community)
        .bind(&alice)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO media_uploaders(community_id, sha256, pubkey)
        SELECT $1, lpad(to_hex(n), 64, '0'), decode($2, 'hex') FROM generate_series(1, 20000) n",
    )
    .bind(community)
    .bind(&bob)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO audit_log(community_id, seq, hash, action, actor_pubkey, object_id)
        SELECT $1, n, decode(lpad(to_hex(n), 64, '0'), 'hex'), 'media_uploaded', decode($2, 'hex'), lpad(to_hex(n + 20000), 64, '0')
        FROM generate_series(1, 20000) n")
        .bind(community).bind(&bob).execute(&pool).await.unwrap();
    // Both discovery sources and both sharing sources have a positive control.
    for n in [1, 20001, 40001] {
        sqlx::query("INSERT INTO media_uploaders(community_id, sha256, pubkey) VALUES ($1, $2, decode($3, 'hex'))")
            .bind(community).bind(format!("{n:064x}")).bind(&alice).execute(&pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO audit_log(community_id, seq, hash, action, actor_pubkey, object_id)
        VALUES ($1, 40002, decode($2, 'hex'), 'media_uploaded', decode($3, 'hex'), $2)",
    )
    .bind(community)
    .bind(format!("{:064x}", 40002))
    .bind(&alice)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql("ANALYZE media_uploaders; ANALYZE audit_log")
        .execute(&pool)
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let actual = db
        .user_media_candidates(std::slice::from_ref(&alice), "", 500)
        .await
        .unwrap();
    assert_eq!(
        actual,
        [1, 20001, 40001, 40002].map(|n| (format!("{n:064x}"), n < 40000))
    );
    // Only a fixed EXPLAIN prefix and the production SQL constant are concatenated.
    let explain = format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {CANDIDATES_SQL}");
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(explain.as_str()))
        .bind(validate_keys(&[alice]).unwrap())
        .bind("")
        .bind(500_i64)
        .fetch_one(&pool)
        .await
        .unwrap();
    fn inspect(node: &Value, indexes: &mut Vec<String>) {
        if node["Actual Loops"].as_f64().unwrap_or(0.0) > 0.0 {
            assert_ne!(
                node["Node Type"], "Seq Scan",
                "unexpected full scan: {node}"
            );
            if let Some(index) = node["Index Name"].as_str() {
                indexes.push(index.to_owned());
            }
        }
        if let Some(children) = node["Plans"].as_array() {
            for child in children {
                inspect(child, indexes);
            }
        }
    }
    let mut indexes = Vec::new();
    inspect(&plan[0]["Plan"], &mut indexes);
    for expected in [
        "idx_media_uploaders_actor_hash",
        "idx_media_uploaders_hash_actor",
        "idx_audit_media_actor_hash",
        "idx_audit_media_hash_actor",
    ] {
        assert!(
            indexes.iter().any(|index| index == expected),
            "missing {expected}: {plan}"
        );
    }
    // Exercise inline creation on a populated table and verification of the
    // already-built indexes. A wrong prebuild must fail, not be accepted by name.
    let migration = include_str!("../../../../../migrations/0089_media_evidence_indexes.sql");
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DROP INDEX idx_media_uploaders_actor_hash")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(migration).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "DROP INDEX idx_media_uploaders_actor_hash;
        CREATE INDEX idx_media_uploaders_actor_hash ON media_uploaders (pubkey)",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    let error = sqlx::raw_sql(migration)
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("invalid or wrong definition"),
        "{error}"
    );
    tx.rollback().await.unwrap();
    println!("media evidence plan: {plan}");
}
