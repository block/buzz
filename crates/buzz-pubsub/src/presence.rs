//! Presence tracking — online/away status with TTL.
//!
//! Stored as `SET buzz:{community}:presence:{pubkey_hex} "online" EX 180`.
//! TTL is 3x the 60s heartbeat interval so a single missed heartbeat doesn't
//! cause presence flap. Clean disconnect deletes immediately.

use buzz_core::TenantContext;
use deadpool_redis::Pool;
use nostr::PublicKey;
use std::collections::HashMap;

use crate::error::PubSubError;
use crate::topic::BUZZ_PREFIX;

/// 3x the 60s heartbeat — single missed heartbeat won't cause presence flap.
pub const PRESENCE_TTL_SECS: u64 = 180;

/// Returns the Redis key for the presence entry of `pubkey` under `ctx`.
pub fn presence_key(ctx: &TenantContext, pubkey: &PublicKey) -> String {
    format!(
        "{BUZZ_PREFIX}:{}:presence:{}",
        ctx.community(),
        pubkey.to_hex()
    )
}

/// Sets presence status for `pubkey` with a [`PRESENCE_TTL_SECS`]-second TTL.
pub async fn set_presence(
    pool: &Pool,
    ctx: &TenantContext,
    pubkey: &PublicKey,
    status: &str,
) -> Result<(), PubSubError> {
    let mut conn = pool.get().await?;
    let key = presence_key(ctx, pubkey);
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
pub async fn clear_presence(
    pool: &Pool,
    ctx: &TenantContext,
    pubkey: &PublicKey,
) -> Result<(), PubSubError> {
    let mut conn = pool.get().await?;
    let key = presence_key(ctx, pubkey);
    redis::cmd("DEL")
        .arg(&key)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Returns the current presence status for `pubkey`, or `None` if not set or expired.
pub async fn get_presence(
    pool: &Pool,
    ctx: &TenantContext,
    pubkey: &PublicKey,
) -> Result<Option<String>, PubSubError> {
    let mut conn = pool.get().await?;
    let key = presence_key(ctx, pubkey);
    let value: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut conn).await?;
    Ok(value)
}

/// Returns `pubkey_hex → status` for all currently-set keys.
///
/// Issues one single-key `GET` per pubkey in a **non-atomic** pipeline: presence
/// keys are per-pubkey, so on a cluster (ElastiCache Serverless included) they
/// span slots and any multi-key command over them fails `CROSSSLOT`.
///
/// **Never call `.atomic()` on this pipeline.** `Pipeline::new` leaves
/// `transaction_mode: false`; `.atomic()` sets it, which makes `query_async`
/// wrap the batch in `MULTI`/`EXEC` and dispatch it as one unit — restoring
/// exactly the cross-slot failure this replaces. A non-atomic pipeline is
/// independent commands sharing one round trip, which is all this read needs:
/// each key is fetched on its own and the entries are TTL'd anyway, so there is
/// nothing for atomicity to protect.
pub async fn get_presence_bulk(
    pool: &Pool,
    ctx: &TenantContext,
    pubkeys: &[PublicKey],
) -> Result<HashMap<String, String>, PubSubError> {
    if pubkeys.is_empty() {
        return Ok(HashMap::new());
    }
    let mut conn = pool.get().await?;
    let mut pipe = redis::pipe();
    for pubkey in pubkeys {
        pipe.cmd("GET").arg(presence_key(ctx, pubkey));
    }
    let values: Vec<Option<String>> = pipe.query_async(&mut conn).await?;

    // Replies are positional — Redis returns values, not key names — so a reply
    // of the wrong length would silently misattribute one pubkey's status to
    // another under `zip`. Refuse instead.
    if values.len() != pubkeys.len() {
        return Err(PubSubError::PresenceReplyMismatch {
            expected: pubkeys.len(),
            got: values.len(),
        });
    }

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
    use buzz_core::{CommunityId, TenantContext};
    use nostr::Keys;
    use uuid::Uuid;

    fn make_pubkey() -> PublicKey {
        Keys::generate().public_key()
    }

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[test]
    fn presence_ttl_is_three_one_minute_heartbeat_windows() {
        assert_eq!(PRESENCE_TTL_SECS, 180);
        assert_eq!(PRESENCE_TTL_SECS, 3 * 60);
    }

    #[test]
    fn test_presence_key_format() {
        let pubkey = make_pubkey();
        let ctx = ctx(0xaaaa, "a.example");
        let key = presence_key(&ctx, &pubkey);
        let prefix = format!("buzz:{}:presence:", ctx.community());
        assert!(key.starts_with(&prefix));
        let hex_part = key.strip_prefix(&prefix).unwrap();
        assert_eq!(hex_part.len(), 64);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn same_pubkey_in_two_communities_has_different_presence_keys() {
        let pubkey = make_pubkey();
        let community_a = ctx(0xaaaa, "a.example");
        let community_b = ctx(0xbbbb, "b.example");

        assert_ne!(
            presence_key(&community_a, &pubkey),
            presence_key(&community_b, &pubkey)
        );
    }

    /// CRC16-XMODEM over the cluster hash-tag-free key, mod 16384 — the Redis
    /// Cluster slot function. Test-local on purpose: production never computes
    /// slots (we use a plain, non-cluster client), this exists only to assert a
    /// key-design property.
    fn key_slot(key: &str) -> u16 {
        let mut crc: u16 = 0;
        for byte in key.as_bytes() {
            crc ^= (*byte as u16) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    (crc << 1) ^ 0x1021
                } else {
                    crc << 1
                };
            }
        }
        crc % 16384
    }

    #[test]
    fn key_slot_matches_redis_cluster_reference_vector() {
        // Redis Cluster spec's CRC16 check value: CRC16-XMODEM("123456789") =
        // 0x31C3, which is below 16384 so the slot equals it outright.
        assert_eq!(key_slot("123456789"), 0x31C3);
    }

    /// Presence keys carry no hash tag, so pubkeys in one community spread across
    /// slots. That is deliberate: tagging them by community would pin an entire
    /// community's presence to a single slot, against AWS's uniform-distribution
    /// guidance and into the per-slot ECPU ceiling. It is also why the bulk read
    /// must be single-key `GET`s rather than any multi-key command.
    #[test]
    fn presence_keys_in_one_community_span_multiple_slots() {
        let ctx = ctx(0xaaaa, "a.example");
        let slots: std::collections::HashSet<u16> = (0..16)
            .map(|_| key_slot(&presence_key(&ctx, &make_pubkey())))
            .collect();
        assert!(
            slots.len() > 1,
            "presence keys must not co-slot; got {slots:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_set_and_get() {
        let pool = make_test_pool();
        let pubkey = make_pubkey();
        let ctx = ctx(0xaaaa, "a.example");

        let status = get_presence(&pool, &ctx, &pubkey).await.unwrap();
        assert!(status.is_none());

        set_presence(&pool, &ctx, &pubkey, "online").await.unwrap();
        let status = get_presence(&pool, &ctx, &pubkey).await.unwrap();
        assert_eq!(status.as_deref(), Some("online"));

        set_presence(&pool, &ctx, &pubkey, "away").await.unwrap();
        let status = get_presence(&pool, &ctx, &pubkey).await.unwrap();
        assert_eq!(status.as_deref(), Some("away"));

        clear_presence(&pool, &ctx, &pubkey).await.unwrap();
        let status = get_presence(&pool, &ctx, &pubkey).await.unwrap();
        assert!(status.is_none());
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_bulk() {
        let pool = make_test_pool();
        let pk1 = make_pubkey();
        let pk2 = make_pubkey();
        let pk3 = make_pubkey();
        let ctx = ctx(0xaaaa, "a.example");

        set_presence(&pool, &ctx, &pk1, "online").await.unwrap();
        set_presence(&pool, &ctx, &pk2, "away").await.unwrap();

        let result = get_presence_bulk(&pool, &ctx, &[pk1, pk2, pk3])
            .await
            .unwrap();

        assert_eq!(
            result.get(&pk1.to_hex()).map(|s| s.as_str()),
            Some("online")
        );
        assert_eq!(result.get(&pk2.to_hex()).map(|s| s.as_str()), Some("away"));
        assert!(!result.contains_key(&pk3.to_hex()));

        clear_presence(&pool, &ctx, &pk1).await.unwrap();
        clear_presence(&pool, &ctx, &pk2).await.unwrap();
    }

    /// Asserts the **reassembled map**, not the reply vector or its length.
    ///
    /// The bulk read zips request positions against reply positions, so a
    /// misalignment is count-correct and attributes one member's status to
    /// another — silent server-side, and user-visible as other people's online
    /// status. Every assertion here is `pubkey → status` for a *distinctly
    /// valued* status, so any permutation of the reply fails.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn presence_bulk_reassembles_by_pubkey_across_slots() {
        let pool = make_test_pool();
        let ctx = ctx(0xb301, "b3.example");

        // Distinct statuses so a shifted reply can't coincidentally match, and
        // enough keys that they span multiple slots (see the slot unit test).
        let present: Vec<(PublicKey, String)> = (0..12)
            .map(|i| (make_pubkey(), format!("status-{i}")))
            .collect();
        let missing: Vec<PublicKey> = (0..4).map(|_| make_pubkey()).collect();

        for (pk, status) in &present {
            set_presence(&pool, &ctx, pk, status).await.unwrap();
        }

        let expected: HashMap<String, String> = present
            .iter()
            .map(|(pk, status)| (pk.to_hex(), status.clone()))
            .collect();

        // Interleave set and unset keys, so a `None` in the middle of the reply
        // must not shift the keys after it.
        let mut requested: Vec<PublicKey> = Vec::new();
        for (i, (pk, _)) in present.iter().enumerate() {
            requested.push(*pk);
            if let Some(absent) = missing.get(i / 3) {
                requested.push(*absent);
            }
        }

        let got = get_presence_bulk(&pool, &ctx, &requested).await.unwrap();
        assert_eq!(got, expected, "reassembled map must be keyed by pubkey");

        // Same set, reversed order: the map is order-independent by definition,
        // so this catches positional misalignment without needing to know which
        // order is "right".
        let mut reversed = requested.clone();
        reversed.reverse();
        let got_reversed = get_presence_bulk(&pool, &ctx, &reversed).await.unwrap();
        assert_eq!(
            got_reversed, expected,
            "map must not depend on request order"
        );

        // Duplicate keys: the reply is one value per request position, and the
        // map must collapse them rather than drift.
        let duplicated: Vec<PublicKey> =
            requested.iter().chain(requested.iter()).copied().collect();
        let got_duplicated = get_presence_bulk(&pool, &ctx, &duplicated).await.unwrap();
        assert_eq!(
            got_duplicated, expected,
            "duplicate requests must collapse to the same map"
        );

        // All-missing: no entries, no error.
        assert!(get_presence_bulk(&pool, &ctx, &missing)
            .await
            .unwrap()
            .is_empty());

        for (pk, _) in &present {
            clear_presence(&pool, &ctx, pk).await.unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn presence_bulk_empty_request_returns_empty_map() {
        let pool = make_test_pool();
        let ctx = ctx(0xb302, "b3.example");
        assert!(get_presence_bulk(&pool, &ctx, &[])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_ttl() {
        let pool = make_test_pool();
        let pubkey = make_pubkey();
        let ctx = ctx(0xaaaa, "a.example");

        set_presence(&pool, &ctx, &pubkey, "online").await.unwrap();

        let mut conn = pool.get().await.unwrap();
        let ttl: i64 = redis::cmd("TTL")
            .arg(presence_key(&ctx, &pubkey))
            .query_async(&mut conn)
            .await
            .unwrap();

        assert!(
            ttl > 0 && ttl <= PRESENCE_TTL_SECS as i64,
            "TTL should be 1-{PRESENCE_TTL_SECS}s, got {ttl}"
        );

        clear_presence(&pool, &ctx, &pubkey).await.unwrap();
    }
}
