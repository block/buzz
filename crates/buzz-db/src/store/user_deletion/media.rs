//! Upload ownership for operator-invoked, offline media cleanup.
use crate::{Db, DbError, Result};
use futures_util::TryStreamExt;
use std::collections::{BTreeMap, BTreeSet};

impl Db {
    /// Read uploader claims across communities once. Copied links are not claims.
    /// Historical uploads with no surviving attribution cannot be assigned.
    pub async fn user_media_uploaders(&self) -> Result<BTreeMap<String, BTreeSet<String>>> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout = '120s'")
            .execute(&mut *tx)
            .await?;
        let mut files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        {
            let mut rows = sqlx::query_as::<_, (String, String)>(
                "SELECT sha256, encode(pubkey,'hex') FROM media_uploaders
                 UNION ALL
                 SELECT object_id, encode(actor_pubkey,'hex') FROM audit_log
                 WHERE action = 'media_uploaded'",
            )
            .fetch(&mut *tx);
            let mut count = 0usize;
            while let Some((hash, uploader)) = rows.try_next().await? {
                count += 1;
                if count > 1_000_000 {
                    return Err(DbError::InvalidData(
                        "media upload scan exceeds one million rows".into(),
                    ));
                }
                if hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(DbError::InvalidData("invalid upload hash".into()));
                }
                let uploader = super::normalize_user_deletion_pubkey(&uploader)?;
                files.entry(hash).or_default().insert(uploader);
            }
        }
        tx.commit().await?;
        Ok(files)
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
