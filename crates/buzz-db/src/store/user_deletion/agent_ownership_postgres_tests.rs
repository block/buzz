//! Shared ownership contract exercised through all four production handlers.
use super::*;
use crate::user_deletion::{
    account_batches::AccountBatchOptions, content_batches::ContentBatchOptions,
    profile_batches::ProfileBatchOptions, ExecutionMode,
};
use nostr::ToBech32;
use serde_json::Value;

struct Fixture {
    db: crate::Db,
    communities: [Uuid; 3],
    human: String,
    other: String,
    agent: String,
}

impl Fixture {
    async fn new() -> Self {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let f = Self {
            db: crate::Db::from_pool(pool),
            communities: std::array::from_fn(|_| Uuid::new_v4()),
            human: nostr::Keys::generate().public_key().to_hex(),
            other: nostr::Keys::generate().public_key().to_hex(),
            agent: nostr::Keys::generate().public_key().to_hex(),
        };
        for community in f.communities {
            sqlx::query("INSERT INTO communities(id,host) VALUES ($1,$2)")
                .bind(community)
                .bind(format!("{community}.example.org"))
                .execute(f.db.pool())
                .await
                .unwrap();
            for key in [&f.human, &f.other] {
                sqlx::query("INSERT INTO users(community_id,pubkey) VALUES ($1,decode($2,'hex'))")
                    .bind(community)
                    .bind(key)
                    .execute(f.db.pool())
                    .await
                    .unwrap();
            }
        }
        for (community, owner) in [(f.communities[0], &f.human), (f.communities[1], &f.other)] {
            // Neither membership nor activation determines ownership.
            sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey,deactivated_at) VALUES ($1,decode($2,'hex'),decode($3,'hex'),now())")
                .bind(community).bind(&f.agent).bind(owner).execute(f.db.pool()).await.unwrap();
        }
        f
    }

    async fn assert_handlers(&self, owned: Option<Uuid>) {
        let store = self.db.user_deletion_store();
        let media = self.db.user_media_targets(&self.human).await.unwrap();
        assert_eq!(
            media.agents,
            owned
                .map(|c| (c, self.agent.clone()))
                .into_iter()
                .collect::<Vec<_>>()
        );
        let scope = store.generate_account_scope(&self.human).await.unwrap();
        for community in self.communities {
            let mut expected = vec![self.human.clone()];
            if Some(community) == owned {
                expected.push(self.agent.clone());
            }
            expected.sort();
            let mut saved = scope
                .communities
                .iter()
                .find(|c| c.community == community)
                .unwrap()
                .pubkeys
                .clone();
            saved.sort();
            assert_eq!(saved, expected);
            let mut output = Vec::new();
            store
                .run_message_batches(
                    &ContentBatchOptions {
                        community,
                        pubkey: self.human.clone(),
                        relay_pubkey: None,
                        batch_size: 100,
                        max_items: 100,
                    },
                    ExecutionMode::ReadOnly,
                    None,
                    &mut output,
                )
                .await
                .unwrap();
            assert_eq!(
                records(&output)[0]["target_pubkeys"],
                serde_json::json!(expected)
            );
            output.clear();
            store
                .run_account_batches(
                    &AccountBatchOptions {
                        community,
                        pubkey: self.human.clone(),
                        batch_size: 100,
                        max_items: 100,
                    },
                    ExecutionMode::ReadOnly,
                    &mut output,
                    None,
                )
                .await
                .unwrap();
            let mut account: Vec<String> = records(&output)
                .into_iter()
                .filter(|r| r["record"] == "account_batches_start")
                .map(|r| r["pubkey"].as_str().unwrap().to_owned())
                .collect();
            account.sort();
            assert_eq!(account, expected);
            let profile = store
                .run_profile_batches(
                    &ProfileBatchOptions {
                        community,
                        pubkey: self.human.clone(),
                        relay_pubkeys: vec![],
                        batch_size: 100,
                        max_items: 100,
                    },
                    false,
                    None,
                    &mut Vec::new(),
                )
                .await
                .unwrap();
            assert_eq!(profile.target_pubkeys, expected);
        }
    }
}

fn records(output: &[u8]) -> Vec<Value> {
    std::str::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn every_handler_discovers_the_same_current_community_agents_on_retry() {
    let f = Fixture::new().await;
    f.assert_handlers(Some(f.communities[0])).await;
    sqlx::query("UPDATE users SET agent_owner_pubkey=CASE WHEN community_id=$1 THEN decode($2,'hex') ELSE decode($3,'hex') END WHERE pubkey=decode($4,'hex')")
        .bind(f.communities[0]).bind(&f.other).bind(&f.human).bind(&f.agent)
        .execute(f.db.pool()).await.unwrap();
    f.assert_handlers(Some(f.communities[1])).await;
    sqlx::query("DELETE FROM users WHERE pubkey=decode($1,'hex')")
        .bind(&f.agent)
        .execute(f.db.pool())
        .await
        .unwrap();
    f.assert_handlers(None).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn discovery_is_direct_normalized_and_holds_the_callers_row_locks() {
    let f = Fixture::new().await;
    let nested = nostr::Keys::generate().public_key().to_hex();
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) VALUES ($1,decode($2,'hex'),decode($3,'hex'))")
        .bind(f.communities[0]).bind(&nested).bind(&f.agent).execute(f.db.pool()).await.unwrap();
    let npub = nostr::PublicKey::parse(&f.human)
        .unwrap()
        .to_bech32()
        .unwrap();
    let mut tx = f.db.pool().begin().await.unwrap();
    let targets = discover(
        &mut tx,
        &npub,
        Some(f.communities[0]),
        true,
        DiscoveryLimit::TargetKeys,
    )
    .await
    .unwrap();
    assert_eq!(targets.human, f.human);
    assert_eq!(targets.agents, vec![(f.communities[0], f.agent.clone())]);
    assert!(!targets
        .agents
        .contains(&(f.communities[1], f.agent.clone())));
    assert!(!targets.agents.contains(&(f.communities[0], nested.clone())));
    let mut transfer = f.db.pool().begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='50ms'")
        .execute(&mut *transfer)
        .await
        .unwrap();
    let error = sqlx::query("UPDATE users SET agent_owner_pubkey=decode($1,'hex') WHERE community_id=$2 AND pubkey=decode($3,'hex')")
        .bind(&f.other).bind(f.communities[0]).bind(&f.agent).execute(&mut *transfer).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("55P03")
    );
    transfer.rollback().await.unwrap();
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn discovery_preserves_each_callers_original_limit_without_truncating_communities() {
    let f = Fixture::new().await;
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) SELECT c,decode(lpad(to_hex(n),64,'0'),'hex'),decode($3,'hex') FROM unnest(ARRAY[$1::uuid,$2::uuid]) c CROSS JOIN generate_series(1,5000) n")
        .bind(f.communities[0]).bind(f.communities[1]).bind(&f.human)
        .execute(f.db.pool()).await.unwrap();
    let media = f.db.user_media_targets(&f.human).await.unwrap();
    assert_eq!(media.agents.len(), 10001);
    assert_eq!(media.pubkeys().len(), 5002);
    assert!(media
        .agents
        .contains(&(f.communities[1], format!("{:064x}", 5000))));
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),decode($2,'hex') FROM generate_series(5001,9998) n")
        .bind(f.communities[0]).bind(&f.human).execute(f.db.pool()).await.unwrap();
    assert_eq!(
        f.db.user_media_targets(&f.human)
            .await
            .unwrap()
            .pubkeys()
            .len(),
        MAX_TARGETS
    );
    let mut tx = f.db.pool().begin().await.unwrap();
    assert_eq!(
        discover(
            &mut tx,
            &f.human,
            Some(f.communities[0]),
            false,
            DiscoveryLimit::TargetKeys
        )
        .await
        .unwrap()
        .pubkeys()
        .len(),
        MAX_TARGETS
    );
    tx.rollback().await.unwrap();
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) VALUES ($1,decode($2,'hex'),decode($3,'hex'))")
        .bind(f.communities[0]).bind(format!("{:064x}", 9999)).bind(&f.human)
        .execute(f.db.pool()).await.unwrap();
    let mut tx = f.db.pool().begin().await.unwrap();
    assert_eq!(
        discover(
            &mut tx,
            &f.human,
            Some(f.communities[0]),
            false,
            DiscoveryLimit::AgentRows
        )
        .await
        .unwrap()
        .agents
        .len(),
        MAX_TARGETS
    );
    for community in [None, Some(f.communities[0])] {
        assert!(discover(
            &mut tx,
            &f.human,
            community,
            false,
            DiscoveryLimit::TargetKeys
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("no partial scope"));
    }
    assert!(
        discover(&mut tx, &f.human, None, false, DiscoveryLimit::AgentRows)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) VALUES ($1,decode($2,'hex'),decode($3,'hex'))")
        .bind(f.communities[0]).bind(format!("{:064x}", 10000)).bind(&f.human)
        .execute(f.db.pool()).await.unwrap();
    let mut tx = f.db.pool().begin().await.unwrap();
    assert!(discover(
        &mut tx,
        &f.human,
        Some(f.communities[0]),
        false,
        DiscoveryLimit::AgentRows
    )
    .await
    .is_err());
    tx.rollback().await.unwrap();
}
