//! Bounded upload evidence queries for offline user media cleanup.
use crate::{Db, DbError, Result};

const MAX_MEDIA_TARGETS: usize = 10_000;

impl Db {
    /// Resolve the human and their directly owned agents once across communities.
    /// Every returned key is a global media target, including its uploads elsewhere.
    pub async fn user_media_targets(&self, pubkey: &str) -> Result<Vec<String>> {
        let pubkey = super::normalize_user_deletion_pubkey(pubkey)?;
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET LOCAL statement_timeout = '120s'")
            .execute(&mut *tx)
            .await?;
        let keys: Vec<String> = sqlx::query_scalar(
            "SELECT key FROM (
                SELECT $1::text AS key
                UNION
                SELECT encode(pubkey, 'hex') FROM users
                WHERE agent_owner_pubkey = decode($1, 'hex')
             ) targets ORDER BY key LIMIT $2",
        )
        .bind(pubkey)
        .bind((MAX_MEDIA_TARGETS + 1) as i64)
        .fetch_all(&mut *tx)
        .await?;
        validate_keys(&keys)?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Page through the target set's distinct upload hashes across communities.
    /// An empty cursor starts the scan. Storage-only claims are discovered separately.
    pub async fn user_media_candidates(
        &self,
        pubkeys: &[String],
        after: &str,
        limit: u16,
    ) -> Result<Vec<String>> {
        let keys = validate_keys(pubkeys)?;
        if !(1..=500).contains(&limit) {
            return Err(DbError::InvalidData(
                "media page size must be 1..=500".into(),
            ));
        }
        if !after.is_empty() {
            validate_hash(after)?;
        }
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET LOCAL statement_timeout = '120s'")
            .execute(&mut *tx)
            .await?;
        let hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT sha256 FROM (
                SELECT sha256 COLLATE "C" AS sha256 FROM media_uploaders
                WHERE pubkey = ANY($1::bytea[]) AND sha256 COLLATE "C" > $2
                UNION
                SELECT object_id COLLATE "C" AS sha256 FROM audit_log
                WHERE action = 'media_uploaded' AND actor_pubkey = ANY($1::bytea[])
                  AND object_id COLLATE "C" > $2
             ) candidates ORDER BY sha256 COLLATE "C" LIMIT $3"#,
        )
        .bind(keys)
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        for hash in &hashes {
            validate_hash(hash)?;
        }
        tx.commit().await?;
        Ok(hashes)
    }

    /// Return only candidate hashes with an uploader outside the target set in either DB source.
    /// Call this for storage-discovered candidates too, before deleting anything.
    pub async fn user_media_shared_hashes(
        &self,
        pubkeys: &[String],
        hashes: &[String],
    ) -> Result<Vec<String>> {
        let keys = validate_keys(pubkeys)?;
        if hashes.len() > 500 {
            return Err(DbError::InvalidData(
                "media evidence batch exceeds 500 hashes".into(),
            ));
        }
        for hash in hashes {
            validate_hash(hash)?;
        }
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET LOCAL statement_timeout = '120s'")
            .execute(&mut *tx)
            .await?;
        let shared = sqlx::query_scalar(
            r#"SELECT candidate FROM unnest($2::text[]) AS candidates(candidate)
             WHERE EXISTS (
                 SELECT 1 FROM media_uploaders
                 WHERE sha256 = candidate AND pubkey <> ALL($1::bytea[])
             ) OR EXISTS (
                 SELECT 1 FROM audit_log
                 WHERE action = 'media_uploaded' AND object_id = candidate
                   AND actor_pubkey <> ALL($1::bytea[])
             ) ORDER BY candidate COLLATE "C""#,
        )
        .bind(keys)
        .bind(hashes)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(shared)
    }

    /// Remove the target set's ownership rows only after storage cleanup succeeds.
    /// Audit history and publication moderation records are retained.
    pub async fn remove_user_media_uploader(&self, pubkeys: &[String], hash: &str) -> Result<u64> {
        let keys = validate_keys(pubkeys)?;
        validate_hash(hash)?;
        Ok(sqlx::query(
            "DELETE FROM media_uploaders WHERE pubkey = ANY($1::bytea[]) AND sha256 = $2",
        )
        .bind(keys)
        .bind(hash)
        .execute(self.pool())
        .await?
        .rows_affected())
    }
}

fn validate_keys(pubkeys: &[String]) -> Result<Vec<Vec<u8>>> {
    if pubkeys.is_empty() || pubkeys.len() > MAX_MEDIA_TARGETS {
        return Err(DbError::InvalidData(
            "media target set must contain 1..=10000 keys".into(),
        ));
    }
    pubkeys
        .iter()
        .map(|key| {
            let key = super::normalize_user_deletion_pubkey(key)?;
            hex::decode(key).map_err(|error| DbError::InvalidData(error.to_string()))
        })
        .collect()
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(DbError::InvalidData("invalid upload hash".into()));
    }
    Ok(())
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn candidates_page_by_user_and_sharing_joins_both_sources() {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let alice = nostr::Keys::generate().public_key().to_hex();
        let bob = nostr::Keys::generate().public_key().to_hex();
        let community = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities(id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("{community}.media.example"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO media_uploaders(community_id, sha256, pubkey)
            SELECT $1, lpad(to_hex(n), 64, '0'), decode($2, 'hex') FROM generate_series(1, 501) n",
        )
        .bind(community)
        .bind(&alice)
        .execute(&pool)
        .await
        .unwrap();
        // Alice's duplicate audit evidence must not duplicate candidate 1.
        // Audit-only candidate 502 is also returned. Bob's unrelated 503 is not.
        for (seq, hash, actor) in [
            (1, 1, &alice),
            (2, 502, &alice),
            (3, 2, &bob),
            (4, 503, &bob),
            (5, 504, &bob),
        ] {
            sqlx::query(
                "INSERT INTO audit_log(community_id, seq, hash, action, actor_pubkey, object_id)
                VALUES ($1, $2, decode($3, 'hex'), 'media_uploaded', decode($4, 'hex'), $5)",
            )
            .bind(community)
            .bind(i64::from(seq))
            .bind(format!("{seq:064x}"))
            .bind(actor)
            .bind(format!("{hash:064x}"))
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query("INSERT INTO media_uploaders(community_id, sha256, pubkey) VALUES ($1, $2, decode($3, 'hex'))")
            .bind(community).bind(format!("{:064x}", 502)).bind(&bob)
            .execute(&pool).await.unwrap();
        let keys = vec![alice.clone()];
        let first = db.user_media_candidates(&keys, "", 500).await.unwrap();
        assert_eq!(first.len(), 500);
        assert_eq!(first[0], format!("{:064x}", 1));
        let second = db
            .user_media_candidates(&keys, &first[499], 500)
            .await
            .unwrap();
        assert_eq!(
            second,
            vec![format!("{:064x}", 501), format!("{:064x}", 502)]
        );
        assert!(db
            .user_media_candidates(&keys, &second[1], 500)
            .await
            .unwrap()
            .is_empty());
        // 504 models a candidate found only in storage for Alice: Bob's DB
        // evidence still protects it although Alice has no DB claim for it.
        let candidates: Vec<_> = [1, 2, 502, 504]
            .into_iter()
            .map(|i| format!("{i:064x}"))
            .collect();
        assert_eq!(
            db.user_media_shared_hashes(&keys, &candidates)
                .await
                .unwrap(),
            candidates[1..]
        );
        assert!(db.user_media_candidates(&keys, "", 501).await.is_err());
        assert!(db
            .user_media_shared_hashes(&keys, &vec!["a".repeat(64); 501])
            .await
            .is_err());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owned_agents_form_one_global_media_target_set() {
        use nostr::nips::nip19::ToBech32;
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let alice_key = nostr::Keys::generate().public_key();
        let alice = alice_key.to_hex();
        let agent = nostr::Keys::generate().public_key().to_hex();
        let bob = nostr::Keys::generate().public_key().to_hex();
        let bobs_agent = nostr::Keys::generate().public_key().to_hex();
        let communities = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()];
        for community in communities {
            sqlx::query("INSERT INTO communities(id, host) VALUES ($1, $2)")
                .bind(community)
                .bind(format!("{community}.media.example"))
                .execute(&pool)
                .await
                .unwrap();
            for person in [&alice, &bob] {
                sqlx::query(
                    "INSERT INTO users(community_id, pubkey) VALUES ($1, decode($2, 'hex'))",
                )
                .bind(community)
                .bind(person)
                .execute(&pool)
                .await
                .unwrap();
            }
            for (key, owner) in [(&agent, &alice), (&bobs_agent, &bob)] {
                sqlx::query(
                    "INSERT INTO users(community_id, pubkey, agent_owner_pubkey, deactivated_at)
                    VALUES ($1, decode($2, 'hex'), decode($3, 'hex'), NOW())",
                )
                .bind(community)
                .bind(key)
                .bind(owner)
                .execute(&pool)
                .await
                .unwrap();
            }
        }
        let keys = db
            .user_media_targets(&alice_key.to_bech32().unwrap())
            .await
            .unwrap();
        let mut expected = vec![alice.clone(), agent.clone()];
        expected.sort();
        assert_eq!(keys, expected); // Duplicate/deactivated profiles still yield one agent key.
        assert_eq!(
            db.user_media_targets(&agent).await.unwrap(),
            vec![agent.clone()]
        );
        let hashes: Vec<_> = (1..=5).map(|n| format!("{n:064x}")).collect();
        for (index, owner, community) in [
            (0, &alice, communities[0]),
            (0, &agent, communities[1]),
            (1, &agent, communities[1]),
            (2, &alice, communities[0]),
            (2, &bob, communities[1]),
            (3, &bobs_agent, communities[0]),
        ] {
            sqlx::query(
                "INSERT INTO media_uploaders(community_id, sha256, pubkey)
                VALUES ($1, $2, decode($3, 'hex'))",
            )
            .bind(community)
            .bind(&hashes[index])
            .bind(owner)
            .execute(&pool)
            .await
            .unwrap();
        }
        // Internal sharing in audit must not protect 1; outside audit protects 2.
        // Agent's audit-only 5 must be discovered.
        for (seq, index, actor) in [(1, 0, &agent), (2, 1, &bob), (3, 4, &agent)] {
            sqlx::query(
                "INSERT INTO audit_log(community_id, seq, hash, action, actor_pubkey, object_id)
                VALUES ($1, $2, decode($3, 'hex'), 'media_uploaded', decode($4, 'hex'), $5)",
            )
            .bind(communities[0])
            .bind(i64::from(seq))
            .bind(format!("{seq:064x}"))
            .bind(actor)
            .bind(&hashes[index])
            .execute(&pool)
            .await
            .unwrap();
        }
        let candidates = vec![
            hashes[0].clone(),
            hashes[1].clone(),
            hashes[2].clone(),
            hashes[4].clone(),
        ];
        assert_eq!(
            db.user_media_candidates(&keys, "", 500).await.unwrap(),
            candidates
        );
        assert_eq!(
            db.user_media_shared_hashes(&keys, &candidates)
                .await
                .unwrap(),
            hashes[1..3]
        );
        assert_eq!(
            db.remove_user_media_uploader(&keys, &hashes[0])
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.remove_user_media_uploader(&keys, &hashes[2])
                .await
                .unwrap(),
            1
        );
        let remaining: Vec<String> = sqlx::query_scalar(
            "SELECT encode(pubkey, 'hex') FROM media_uploaders WHERE sha256 = $1",
        )
        .bind(&hashes[2])
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining, vec![bob]);
        assert_eq!(
            db.remove_user_media_uploader(&keys, &hashes[0])
                .await
                .unwrap(),
            0
        );
        // Retained upload history continues to support retries.
        assert!(db
            .user_media_candidates(&keys, "", 500)
            .await
            .unwrap()
            .contains(&hashes[0]));
        assert!(db.user_media_candidates(&[], "", 500).await.is_err());
        assert!(db
            .remove_user_media_uploader(&["invalid".into()], &hashes[0])
            .await
            .is_err());
    }
}
