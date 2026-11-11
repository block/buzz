//! Bounded upload evidence queries for offline user media cleanup.
use crate::{Db, DbError, Result};

impl Db {
    /// Page through this person's distinct upload hashes across communities.
    /// An empty cursor starts the scan. Storage-only claims are discovered separately.
    pub async fn user_media_candidates(
        &self,
        pubkey: &str,
        after: &str,
        limit: u16,
    ) -> Result<Vec<String>> {
        let pubkey = super::normalize_user_deletion_pubkey(pubkey)?;
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
                WHERE pubkey = decode($1, 'hex') AND sha256 COLLATE "C" > $2
                UNION
                SELECT object_id COLLATE "C" AS sha256 FROM audit_log
                WHERE action = 'media_uploaded' AND actor_pubkey = decode($1, 'hex')
                  AND object_id COLLATE "C" > $2
             ) candidates ORDER BY sha256 COLLATE "C" LIMIT $3"#,
        )
        .bind(pubkey)
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

    /// Return only candidate hashes with another uploader in either DB source.
    /// Call this for storage-discovered candidates too, before deleting anything.
    pub async fn user_media_shared_hashes(
        &self,
        pubkey: &str,
        hashes: &[String],
    ) -> Result<Vec<String>> {
        let pubkey = super::normalize_user_deletion_pubkey(pubkey)?;
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
                 WHERE sha256 = candidate AND pubkey <> decode($1, 'hex')
             ) OR EXISTS (
                 SELECT 1 FROM audit_log
                 WHERE action = 'media_uploaded' AND object_id = candidate
                   AND actor_pubkey <> decode($1, 'hex')
             ) ORDER BY candidate COLLATE "C""#,
        )
        .bind(pubkey)
        .bind(hashes)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(shared)
    }

    /// Remove this person's ownership rows only after storage cleanup succeeds.
    /// Audit history and publication moderation records are retained.
    pub async fn remove_user_media_uploader(&self, pubkey: &str, hash: &str) -> Result<u64> {
        Ok(sqlx::query(
            "DELETE FROM media_uploaders WHERE pubkey = decode($1,'hex') AND sha256 = $2",
        )
        .bind(pubkey)
        .bind(hash)
        .execute(self.pool())
        .await?
        .rows_affected())
    }
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
        let first = db.user_media_candidates(&alice, "", 500).await.unwrap();
        assert_eq!(first.len(), 500);
        assert_eq!(first[0], format!("{:064x}", 1));
        let second = db
            .user_media_candidates(&alice, &first[499], 500)
            .await
            .unwrap();
        assert_eq!(
            second,
            vec![format!("{:064x}", 501), format!("{:064x}", 502)]
        );
        assert!(db
            .user_media_candidates(&alice, &second[1], 500)
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
            db.user_media_shared_hashes(&alice, &candidates)
                .await
                .unwrap(),
            candidates[1..]
        );
        assert!(db.user_media_candidates(&alice, "", 501).await.is_err());
        assert!(db
            .user_media_shared_hashes(&alice, &vec!["a".repeat(64); 501])
            .await
            .is_err());
    }
}
