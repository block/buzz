//! Presence tracking — online/away status with TTL.
//!
//! Stored as `SET sprout:presence:{pubkey_hex} "online" EX 90`.
//! TTL is 3x the 30s heartbeat interval so a single missed heartbeat doesn't
//! cause presence flap. Clean disconnect deletes immediately.

use deadpool_redis::Pool;
use nostr::PublicKey;
use std::collections::HashMap;

use crate::error::PubSubError;

/// 3x the 30s heartbeat — single missed heartbeat won't cause presence flap.
pub const PRESENCE_TTL_SECS: u64 = 90;

/// Returns the Redis key for the presence entry of `pubkey`.
pub fn presence_key(pubkey: &PublicKey) -> String {
    format!("sprout:presence:{}", pubkey.to_hex())
}

/// Sets presence status for `pubkey` with a [`PRESENCE_TTL_SECS`]-second TTL.
pub async fn set_presence(
    pool: &Pool,
    pubkey: &PublicKey,
    status: &str,
) -> Result<(), PubSubError> {
    let mut conn = pool.get().await?;
    let key = presence_key(pubkey);
    redis::cmd("SET")
        .arg(&key)
        .arg(status)
        .arg("EX")
        .arg(PRESENCE_TTL_SECS)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Removes the presence entry for `pubkey`. Call on clean disconnect.
pub async fn clear_presence(pool: &Pool, pubkey: &PublicKey) -> Result<(), PubSubError> {
    let mut conn = pool.get().await?;
    let key = presence_key(pubkey);
    redis::cmd("DEL")
        .arg(&key)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Returns the current presence status for `pubkey`, or `None` if not set or expired.
pub async fn get_presence(pool: &Pool, pubkey: &PublicKey) -> Result<Option<String>, PubSubError> {
    let mut conn = pool.get().await?;
    let key = presence_key(pubkey);
    let value: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
    Ok(value)
}

/// Returns `pubkey_hex → status` for all currently-set keys.
pub async fn get_presence_bulk(
    pool: &Pool,
    pubkeys: &[PublicKey],
) -> Result<HashMap<String, String>, PubSubError> {
    if pubkeys.is_empty() {
        return Ok(HashMap::new());
    }
    let mut conn = pool.get().await?;
    let keys: Vec<String> = pubkeys.iter().map(presence_key).collect();
    let values: Vec<Option<String>> = redis::cmd("MGET").arg(&keys).query_async(&mut conn).await?;
    let result = pubkeys
        .iter()
        .zip(values.iter())
        .filter_map(|(pk, v)| v.as_ref().map(|s| (pk.to_hex(), s.clone())))
        .collect();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::make_test_pool;
    use nostr::Keys;

    fn make_pubkey() -> PublicKey {
        Keys::generate().public_key()
    }

    #[test]
    fn test_presence_key_format() {
        let pubkey = make_pubkey();
        let key = presence_key(&pubkey);
        assert!(key.starts_with("sprout:presence:"));
        let hex_part = key.strip_prefix("sprout:presence:").unwrap();
        assert_eq!(hex_part.len(), 64);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_set_and_get() {
        let pool = make_test_pool();
        let pubkey = make_pubkey();

        let status = get_presence(&pool, &pubkey).await.unwrap();
        assert!(status.is_none());

        set_presence(&pool, &pubkey, "online").await.unwrap();
        let status = get_presence(&pool, &pubkey).await.unwrap();
        assert_eq!(status.as_deref(), Some("online"));

        set_presence(&pool, &pubkey, "away").await.unwrap();
        let status = get_presence(&pool, &pubkey).await.unwrap();
        assert_eq!(status.as_deref(), Some("away"));

        clear_presence(&pool, &pubkey).await.unwrap();
        let status = get_presence(&pool, &pubkey).await.unwrap();
        assert!(status.is_none());
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_bulk() {
        let pool = make_test_pool();
        let pk1 = make_pubkey();
        let pk2 = make_pubkey();
        let pk3 = make_pubkey();

        set_presence(&pool, &pk1, "online").await.unwrap();
        set_presence(&pool, &pk2, "away").await.unwrap();

        let result = get_presence_bulk(&pool, &[pk1, pk2, pk3]).await.unwrap();

        assert_eq!(
            result.get(&pk1.to_hex()).map(|s| s.as_str()),
            Some("online")
        );
        assert_eq!(result.get(&pk2.to_hex()).map(|s| s.as_str()), Some("away"));
        assert!(!result.contains_key(&pk3.to_hex()));

        clear_presence(&pool, &pk1).await.unwrap();
        clear_presence(&pool, &pk2).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_ttl() {
        let pool = make_test_pool();
        let pubkey = make_pubkey();

        set_presence(&pool, &pubkey, "online").await.unwrap();

        let mut conn = pool.get().await.unwrap();
        let ttl: i64 = redis::cmd("TTL")
            .arg(presence_key(&pubkey))
            .query_async(&mut conn)
            .await
            .unwrap();

        assert!(
            ttl > 0 && ttl <= PRESENCE_TTL_SECS as i64,
            "TTL should be 1-{PRESENCE_TTL_SECS}s, got {ttl}"
        );

        clear_presence(&pool, &pubkey).await.unwrap();
    }
}
