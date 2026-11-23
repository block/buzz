//! Same agent identity has independent ownership in each community.
use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_ownership_controls_candidates_sharing_removal_and_fresh_retry() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let alice = nostr::Keys::generate().public_key().to_hex();
    let bob = nostr::Keys::generate().public_key().to_hex();
    let agent = nostr::Keys::generate().public_key().to_hex();
    let nested_agent = nostr::Keys::generate().public_key().to_hex();
    let communities = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    for community in communities {
        sqlx::query("INSERT INTO communities(id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("{community}.example.org"))
            .execute(&pool)
            .await
            .unwrap();
    }
    for community in communities {
        for key in [&alice, &bob] {
            sqlx::query("INSERT INTO users(community_id, pubkey) VALUES ($1, decode($2, 'hex'))")
                .bind(community)
                .bind(key)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    for (community, key, owner) in [
        (communities[0], &agent, &alice),
        (communities[1], &agent, &bob),
        (communities[0], &nested_agent, &agent),
    ] {
        sqlx::query("INSERT INTO users(community_id, pubkey, agent_owner_pubkey) VALUES ($1, decode($2, 'hex'), decode($3, 'hex'))")
            .bind(community).bind(key).bind(owner).execute(&pool).await.unwrap();
    }
    let hashes: Vec<_> = (1..=7).map(|n| format!("{n:064x}")).collect();
    for (community, index, key) in [
        (0, 0, &agent),
        (1, 0, &agent), // Conflicting community protects shared blob.
        (0, 1, &agent),
        (2, 1, &agent),        // Missing profile also protects shared blob.
        (1, 2, &agent),        // Outside-only upload is not a candidate.
        (0, 3, &agent),        // Exclusive owned upload is selected.
        (0, 4, &nested_agent), // Direct ownership only; no recursive expansion.
        (0, 5, &alice),        // Outside audit evidence protects this human upload.
    ] {
        sqlx::query("INSERT INTO media_uploaders(community_id, sha256, pubkey) VALUES ($1, $2, decode($3, 'hex'))")
            .bind(communities[community]).bind(&hashes[index]).bind(key).execute(&pool).await.unwrap();
    }
    for (seq, community, index) in [(1_i64, 0, 6), (2, 1, 5), (3, 1, 2), (4, 2, 1)] {
        sqlx::query(
            "INSERT INTO audit_log(community_id, seq, hash, action, actor_pubkey, object_id)
            VALUES ($1, $2, decode($3, 'hex'), 'media_uploaded', decode($4, 'hex'), $5)",
        )
        .bind(communities[community])
        .bind(seq)
        .bind(format!("{seq:064x}"))
        .bind(&agent)
        .bind(&hashes[index])
        .execute(&pool)
        .await
        .unwrap();
    }
    let targets = db.user_media_targets(&alice).await.unwrap();
    assert_eq!(targets.agents, vec![(communities[0], agent.clone())]);
    assert_eq!(
        db.user_media_candidates(&targets, "", 500).await.unwrap(),
        [0, 1, 3, 5, 6].map(|i| (hashes[i].clone(), [0, 1, 5].contains(&i)))
    );
    assert_eq!(
        db.user_media_shared_hashes(&targets, &hashes)
            .await
            .unwrap(),
        [0, 1, 2, 4, 5].map(|i| hashes[i].clone())
    );
    for index in [0, 1] {
        assert_eq!(
            db.remove_user_media_uploader(&targets, &hashes[index])
                .await
                .unwrap(),
            1
        );
        let remaining: Vec<Uuid> =
            sqlx::query_scalar("SELECT community_id FROM media_uploaders WHERE sha256 = $1")
                .bind(&hashes[index])
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec![communities[index + 1]]);
    }
    // Ownership transfers between runs; the same pubkey list must not hide the changed community.
    sqlx::query("UPDATE users SET agent_owner_pubkey = CASE WHEN community_id = $1 THEN decode($2, 'hex') ELSE decode($3, 'hex') END WHERE pubkey = decode($4, 'hex')")
        .bind(communities[0]).bind(&bob).bind(&alice).bind(&agent).execute(&pool).await.unwrap();
    let retry = db.user_media_targets(&alice).await.unwrap();
    assert_eq!(retry.pubkeys(), targets.pubkeys());
    assert_eq!(retry.agents, vec![(communities[1], agent.clone())]);
    let candidates = db.user_media_candidates(&retry, "", 500).await.unwrap();
    assert!(candidates.iter().any(|(hash, _)| hash == &hashes[2]));
    assert!(!candidates
        .iter()
        .any(|(hash, _)| hash == &hashes[3] || hash == &hashes[6]));
    // Removing ownership evidence excludes the agent on the next invocation.
    sqlx::query("DELETE FROM users WHERE pubkey IN (decode($1, 'hex'), decode($2, 'hex'))")
        .bind(&agent)
        .bind(&nested_agent)
        .execute(&pool)
        .await
        .unwrap();
    assert!(db
        .user_media_targets(&alice)
        .await
        .unwrap()
        .agents
        .is_empty());
}
