//! APNs token custody. Ciphertexts are self-describing and can be decrypted by
//! the current key or bounded decrypt-only predecessors during key rotation.
use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

const AAD_PREFIX: &[u8] = b"buzz-apns-token-v1:";
const MAX_KEY_ID_BYTES: usize = 32;
const MAX_CIPHERTEXT_BYTES: usize = 2048;

#[derive(Clone)]
pub struct TokenKey {
    id: String,
    cipher: Aes256Gcm,
}
#[derive(Clone)]
pub struct TokenKeyring {
    current: TokenKey,
    predecessors: HashMap<String, TokenKey>,
}

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("invalid token ciphertext")]
    Invalid,
    #[error("token keyring is empty or contains duplicate ids")]
    InvalidKeyring,
    #[error("stored APNs token cannot be decrypted with configured storage keys")]
    UnreadableStorage,
}

impl TokenKey {
    pub fn new(id: impl Into<String>, key: &[u8]) -> Result<Self, TokenError> {
        let id = id.into();
        if id.is_empty()
            || id.len() > MAX_KEY_ID_BYTES
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(TokenError::Invalid);
        }
        Ok(Self {
            id,
            cipher: Aes256Gcm::new_from_slice(key).map_err(|_| TokenError::Invalid)?,
        })
    }
    fn aad(&self) -> Vec<u8> {
        [AAD_PREFIX, self.id.as_bytes()].concat()
    }
}

impl TokenKeyring {
    pub fn new(keys: Vec<TokenKey>) -> Result<Self, TokenError> {
        let mut keys = keys.into_iter();
        let current = keys.next().ok_or(TokenError::InvalidKeyring)?;
        let rest: Vec<_> = keys.collect();
        let mut ids = HashSet::new();
        if !ids.insert(current.id.clone()) || rest.iter().any(|k| !ids.insert(k.id.clone())) {
            return Err(TokenError::InvalidKeyring);
        }
        Ok(Self {
            current,
            predecessors: rest.into_iter().map(|k| (k.id.clone(), k)).collect(),
        })
    }
    pub fn seal(&self, token: &[u8]) -> Result<Vec<u8>, TokenError> {
        if token.is_empty() || token.len() > crate::model::MAX_ENDPOINT_HEX_BYTES {
            return Err(TokenError::Invalid);
        }
        let mut nonce = [0; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut sealed = nonce.to_vec();
        sealed.extend(
            self.current
                .cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    aes_gcm::aead::Payload {
                        msg: token,
                        aad: &self.current.aad(),
                    },
                )
                .map_err(|_| TokenError::Invalid)?,
        );
        let encoded =
            format!("{}.{}", self.current.id, URL_SAFE_NO_PAD.encode(sealed)).into_bytes();
        if encoded.len() > MAX_CIPHERTEXT_BYTES {
            return Err(TokenError::Invalid);
        }
        Ok(encoded)
    }
    pub fn open(&self, encoded: &[u8]) -> Result<Vec<u8>, TokenError> {
        if encoded.len() > MAX_CIPHERTEXT_BYTES {
            return Err(TokenError::Invalid);
        }
        let encoded = std::str::from_utf8(encoded).map_err(|_| TokenError::Invalid)?;
        let (id, body) = encoded.split_once('.').ok_or(TokenError::Invalid)?;
        let key = if id == self.current.id {
            &self.current
        } else {
            self.predecessors.get(id).ok_or(TokenError::Invalid)?
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| TokenError::Invalid)?;
        if bytes.len() < 13 {
            return Err(TokenError::Invalid);
        }
        key.cipher
            .decrypt(
                Nonce::from_slice(&bytes[..12]),
                aes_gcm::aead::Payload {
                    msg: &bytes[12..],
                    aad: &key.aad(),
                },
            )
            .map_err(|_| TokenError::Invalid)
    }

    /// Verify every retained token is readable, including revocation tombstones.
    /// Return the number that still use a predecessor key.
    pub async fn audit_storage(
        &self,
        pool: &sqlx::PgPool,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        self.scan_storage(pool, false).await
    }

    /// Re-encrypt a finite snapshot in bounded batches. Concurrent endpoint
    /// changes win; callers must audit again before retiring any reader key.
    pub async fn reencrypt_storage(
        &self,
        pool: &sqlx::PgPool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.scan_storage(pool, true).await?;
        if self.audit_storage(pool).await? != 0 {
            return Err("APNs token re-encryption incomplete; drain old writers and rerun".into());
        }
        Ok(())
    }

    async fn scan_storage(
        &self,
        pool: &sqlx::PgPool,
        rewrite: bool,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let cutoff: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT now()").fetch_one(pool).await?;
        let upper: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT id FROM push_gateway_installations ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;
        let Some(upper) = upper else { return Ok(0) };
        let mut after: Option<uuid::Uuid> = None;
        let mut predecessors = 0;
        let current_prefix = format!("{}.", self.current.id);
        loop {
            let rows: Vec<(uuid::Uuid, Vec<u8>, i64, Vec<u8>)> = sqlx::query_as(
                "SELECT id, token_ciphertext, endpoint_epoch, token_fingerprint
                 FROM push_gateway_installations WHERE ($1::uuid IS NULL OR id > $1) AND id <= $2 AND created_at <= $3
                 ORDER BY id LIMIT 128",
            ).bind(after).bind(upper).bind(cutoff).fetch_all(pool).await?;
            if rows.is_empty() {
                break;
            }
            for (id, ciphertext, epoch, fingerprint) in rows {
                after = Some(id);
                let token = self
                    .open(&ciphertext)
                    .map_err(|_| TokenError::UnreadableStorage)?;
                if !ciphertext.starts_with(current_prefix.as_bytes()) {
                    predecessors += 1;
                    if rewrite {
                        replace_ciphertext(
                            pool,
                            id,
                            &ciphertext,
                            epoch,
                            &fingerprint,
                            &self.seal(&token)?,
                        )
                        .await?;
                    }
                }
            }
        }
        Ok(predecessors)
    }
}

async fn replace_ciphertext(
    pool: &sqlx::PgPool,
    id: uuid::Uuid,
    expected: &[u8],
    epoch: i64,
    fingerprint: &[u8],
    replacement: &[u8],
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE push_gateway_installations SET token_ciphertext=$1
         WHERE id=$2 AND token_ciphertext=$3 AND endpoint_epoch=$4 AND token_fingerprint=$5",
    )
    .bind(replacement)
    .bind(id)
    .bind(expected)
    .bind(epoch)
    .bind(fingerprint)
    .execute(pool)
    .await?
    .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(keys: &[(&str, u8)]) -> TokenKeyring {
        TokenKeyring::new(
            keys.iter()
                .map(|(id, byte)| TokenKey::new(*id, &[*byte; 32]).unwrap())
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn ciphertext_readers_precede_writer_switch() {
        let old = ring(&[("old", 1)]);
        let readers = ring(&[("old", 1), ("new", 2)]);
        let writers = ring(&[("new", 2), ("old", 1)]);
        let endpoint = [7; 32];
        let old_ciphertext = old.seal(&endpoint).unwrap();
        let new_ciphertext = writers.seal(&endpoint).unwrap();
        assert_eq!(writers.open(&old_ciphertext).unwrap(), endpoint);
        assert_eq!(readers.open(&new_ciphertext).unwrap(), endpoint);
        assert!(old.open(&new_ciphertext).is_err());
        assert!(ring(&[("new", 3)]).open(&new_ciphertext).is_err());
    }

    mod postgres_tests {
        use super::*;

        #[tokio::test]
        #[ignore = "requires PostgreSQL"]
        async fn storage_rotation_preserves_endpoint_changes_and_guards_retirement() {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect(&std::env::var("BUZZ_TEST_DATABASE_URL").expect("test database URL"))
                .await
                .unwrap();
            let schema = format!("token_rotation_{}", uuid::Uuid::new_v4().simple());
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA {schema}; SET search_path TO {schema}"
            )))
            .execute(&pool)
            .await
            .unwrap();
            sqlx::raw_sql(include_str!(
                "../../../schema/tables/public/push_gateway_installations.sql"
            ))
            .execute(&pool)
            .await
            .unwrap();
            let id = uuid::Uuid::new_v4();
            let old = ring(&[("old", 1)]);
            let rotated = ring(&[("new", 2), ("old", 1)]);
            let retired = ring(&[("new", 2)]);
            let ciphertext = old.seal(&[7; 32]).unwrap();
            // Retained revoked rows need reader keys too; lifetime is irrelevant.
            sqlx::query("INSERT INTO push_gateway_installations(id,app_attest_key_id,app_attest_public_key,assertion_counter,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at,revoked_at) VALUES($1,$2,$3,0,$4,$5,1,now()-interval '1 day',now())")
                .bind(id).bind(vec![1_u8]).bind(vec![2_u8;33]).bind(&ciphertext).bind(vec![3_u8;32])
                .execute(&pool).await.unwrap();
            assert!(retired.audit_storage(&pool).await.is_err());
            assert_eq!(rotated.audit_storage(&pool).await.unwrap(), 1);
            assert!(ring(&[("old", 9)]).audit_storage(&pool).await.is_err());
            let replacement = rotated.seal(&[8; 32]).unwrap();
            // Endpoint rotation wins over maintenance's previously read snapshot.
            sqlx::query("UPDATE push_gateway_installations SET token_ciphertext=$1,endpoint_epoch=2,token_fingerprint=$2 WHERE id=$3")
                .bind(&replacement).bind(vec![4_u8;32]).bind(id).execute(&pool).await.unwrap();
            assert_eq!(
                replace_ciphertext(
                    &pool,
                    id,
                    &ciphertext,
                    1,
                    &[3; 32],
                    &rotated.seal(&[7; 32]).unwrap()
                )
                .await
                .unwrap(),
                0
            );
            let stored: Vec<u8> = sqlx::query_scalar(
                "SELECT token_ciphertext FROM push_gateway_installations WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(stored, replacement);
            // A remaining old ciphertext is rewritten by the production workflow.
            sqlx::query("UPDATE push_gateway_installations SET token_ciphertext=$1 WHERE id=$2")
                .bind(old.seal(&[8; 32]).unwrap())
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
            rotated.reencrypt_storage(&pool).await.unwrap();
            assert_eq!(retired.audit_storage(&pool).await.unwrap(), 0);
            let stored: Vec<u8> = sqlx::query_scalar(
                "SELECT token_ciphertext FROM push_gateway_installations WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(retired.open(&stored).unwrap(), [8; 32]);
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "SET search_path TO public; DROP SCHEMA {schema} CASCADE"
            )))
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        }
    }
}
