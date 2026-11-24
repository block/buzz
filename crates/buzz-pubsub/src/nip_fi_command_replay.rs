//! Redis-backed NIP-FI command replay seen-set and durable deny store.
//!
//! Implements [`CommandReplayGuard`] from `buzz-auth`.  One Lua script checks
//! replay and the issuer's shared capacity, and only when both pass sets the
//! claim (`SET … EX`, same TTL clamp as [`crate::RedisNip98ReplayGuard`]) and
//! records the deny entry.  Per issuer, entries live in three keys that expire
//! with their latest entry:
//!
//! * `{deny}:{sha256(iss)}` — sorted set, member `pubkey_hex`, score `until`
//!   in ms rounded **up**: the index for expiry and capacity.  An entry is
//!   removed only once its score is below `now`, so never early.
//! * `{deny}-until:{sha256(iss)}` — hash, field `pubkey_hex`, value the exact
//!   signed `until` as `secs:nanos`, merged by maximum.
//! * `{deny}-revoke-through:{sha256(iss)}` — hash, field `pubkey_hex`: never
//!   written here; every accept clears its target's field so a full revoke
//!   is never narrowed by a cutoff another relay version stored.
//!
//! Only the issuer's hash is stored; readers map it back through configured
//! issuers.

use buzz_auth::nip_fi::{
    namespaced_command_replay_key, COMMAND_REPLAY_PREFIX, SHADOW_COMMAND_REPLAY_PREFIX,
};
use buzz_auth::{
    error::AuthError, ClaimOutcome, CommandClaim, CommandReplayGuard, LoadDeniesError,
    DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS,
};
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use sha2::{Digest, Sha256};
use std::{future::Future, pin::Pin};

/// Deny-store key prefix for enforce-mode accepts.
pub const DENY_STORE_PREFIX: &str = "buzz:nip-fi:deny";

/// Deny-store key prefix for shadow-mode accepts, disjoint from enforce.
pub const SHADOW_DENY_STORE_PREFIX: &str = "buzz:nip-fi:shadow-deny";

/// KEYS: claim, index, exact, cutoff.  ARGV: ttl, pubkey_hex, score_ms, secs, nanos,
/// capacity, now_ms, active (0/1).
const ACCEPT_SCRIPT: &str = r"
if redis.call('EXISTS', KEYS[1]) == 1 then return 'replay' end
local active = ARGV[8] == '1'
if active then
  local expired = redis.call('ZRANGEBYSCORE', KEYS[2], '-inf', '(' .. ARGV[7])
  for _, member in ipairs(expired) do
    redis.call('HDEL', KEYS[3], member)
    redis.call('HDEL', KEYS[4], member)
  end
  redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', '(' .. ARGV[7])
  if not redis.call('ZSCORE', KEYS[2], ARGV[2])
     and redis.call('ZCARD', KEYS[2]) >= tonumber(ARGV[6]) then
    return 'full'
  end
end
redis.call('SET', KEYS[1], '1', 'EX', ARGV[1])
if active then
  redis.call('ZADD', KEYS[2], 'GT', ARGV[3], ARGV[2])
  local cur = redis.call('HGET', KEYS[3], ARGV[2])
  local keep = false
  if cur then
    local s, n = string.match(cur, '^(%-?%d+):(%d+)$')
    s, n = tonumber(s), tonumber(n)
    local ns, nn = tonumber(ARGV[4]), tonumber(ARGV[5])
    keep = s > ns or (s == ns and n >= nn)
  end
  if not keep then redis.call('HSET', KEYS[3], ARGV[2], ARGV[4] .. ':' .. ARGV[5]) end
  redis.call('HDEL', KEYS[4], ARGV[2])
  local top = redis.call('ZRANGE', KEYS[2], -1, -1, 'WITHSCORES')
  redis.call('PEXPIREAT', KEYS[2], top[2])
  redis.call('PEXPIREAT', KEYS[3], top[2])
  redis.call('PEXPIREAT', KEYS[4], top[2])
end
return 'claimed'
";

/// Redis-backed NIP-FI command replay seen-set and deny store.
pub struct RedisCommandReplayGuard {
    pool: crate::RedisConnection,
    prefix: std::borrow::Cow<'static, str>,
    deny_prefix: std::borrow::Cow<'static, str>,
}

impl RedisCommandReplayGuard {
    /// Enforce-mode claims and deny records.
    pub fn new(pool: crate::RedisConnection) -> Self {
        Self {
            pool,
            prefix: COMMAND_REPLAY_PREFIX.into(),
            deny_prefix: DENY_STORE_PREFIX.into(),
        }
    }

    /// Shadow-mode claims and deny records, disjoint from enforce.
    pub fn shadow(pool: crate::RedisConnection) -> Self {
        Self {
            pool,
            prefix: SHADOW_COMMAND_REPLAY_PREFIX.into(),
            deny_prefix: SHADOW_DENY_STORE_PREFIX.into(),
        }
    }

    /// Claims and deny records under `namespace`, disjoint from both modes.
    /// For tests that must not share capacity with other cases on one Redis.
    #[doc(hidden)]
    pub fn namespaced(pool: crate::RedisConnection, namespace: &str) -> Self {
        Self {
            pool,
            prefix: format!("{namespace}:command").into(),
            deny_prefix: format!("{namespace}:deny").into(),
        }
    }
}

/// Index (sorted set) and exact-`until` (hash) keys for `issuer` under `prefix`.
pub fn deny_store_keys(prefix: &str, issuer: &str) -> (String, String) {
    let hash = hex::encode(Sha256::digest(issuer.as_bytes()));
    (format!("{prefix}:{hash}"), format!("{prefix}-until:{hash}"))
}

/// Per-target revoke-through cutoff hash for `issuer` under `prefix`.
pub fn revoke_through_key(prefix: &str, issuer: &str) -> String {
    let hash = hex::encode(Sha256::digest(issuer.as_bytes()));
    format!("{prefix}-revoke-through:{hash}")
}

/// `until` in whole ms, rounded up.
fn ceil_ms(t: DateTime<Utc>) -> i64 {
    let ms = t.timestamp_millis();
    if t.timestamp_subsec_nanos().is_multiple_of(1_000_000) {
        ms
    } else {
        ms + 1
    }
}

fn internal(e: impl std::fmt::Display) -> AuthError {
    tracing::warn!(error = %e, "nip-fi command store: redis failed");
    AuthError::Internal(format!("nip-fi command store: {e}"))
}

impl CommandReplayGuard for RedisCommandReplayGuard {
    fn try_claim<'a>(
        &'a self,
        claim: CommandClaim<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ClaimOutcome, AuthError>> + Send + 'a>> {
        Box::pin(async move {
            let ttl = claim
                .ttl_secs
                .clamp(DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS);
            let (index, exact) = deny_store_keys(&self.deny_prefix, claim.issuer);
            let mut conn = self.pool.clone();
            let reply: String = redis::Script::new(ACCEPT_SCRIPT)
                .key(namespaced_command_replay_key(
                    &self.prefix,
                    claim.issuer,
                    claim.jti,
                ))
                .key(index)
                .key(exact)
                .key(revoke_through_key(&self.deny_prefix, claim.issuer))
                .arg(ttl)
                .arg(claim.target.to_hex())
                .arg(ceil_ms(claim.until))
                .arg(claim.until.timestamp())
                .arg(claim.until.timestamp_subsec_nanos())
                .arg(claim.capacity)
                .arg(claim.now.timestamp_millis())
                .arg(u8::from(claim.until > claim.now))
                .invoke_async(&mut conn)
                .await
                .map_err(internal)?;
            match reply.as_str() {
                "claimed" => Ok(ClaimOutcome::Claimed),
                "replay" => Ok(ClaimOutcome::Replay),
                "full" => Ok(ClaimOutcome::Full),
                other => Err(internal(format!("unexpected reply: {other}"))),
            }
        })
    }

    fn load_denies<'a>(&'a self, issuer: &'a str, now: DateTime<Utc>) -> buzz_auth::LoadDenies<'a> {
        Box::pin(async move {
            let (_, exact) = deny_store_keys(&self.deny_prefix, issuer);
            let mut conn = self.pool.clone();
            let entries: Vec<(Vec<u8>, Vec<u8>)> = redis::cmd("HGETALL")
                .arg(exact)
                .query_async(&mut conn)
                .await
                .map_err(internal)?;
            // An unreadable record fails the load: skipping it would reopen
            // admission without that revocation.  Every other record is
            // still decoded, so the caller can apply them meanwhile.
            let mut denies = Vec::with_capacity(entries.len());
            let mut bad = None;
            for (key, until) in entries {
                match decode(key, until) {
                    Ok((key, until)) if until > now => denies.push((key, until)),
                    Ok(_) => {}
                    Err(record) => {
                        tracing::warn!(record, "nip-fi command store: unreadable deny record");
                        bad.get_or_insert(record);
                    }
                }
            }
            match bad {
                None => Ok(denies),
                Some(record) => Err(LoadDeniesError::Unreadable {
                    record,
                    readable: denies,
                }),
            }
        })
    }
}

fn parse_until(value: &str) -> Option<DateTime<Utc>> {
    let (secs, nanos) = value.split_once(':')?;
    DateTime::from_timestamp(secs.parse().ok()?, nanos.parse().ok()?)
}

/// One `HGETALL` field and value as a deny, or the record that won't decode.
fn decode(key: Vec<u8>, until: Vec<u8>) -> Result<(PublicKey, DateTime<Utc>), String> {
    let (Ok(key), Ok(until)) = (String::from_utf8(key), String::from_utf8(until)) else {
        return Err("deny record that is not UTF-8".into());
    };
    let key = PublicKey::from_hex(&key).map_err(|_| format!("deny key {key}"))?;
    let until = parse_until(&until).ok_or_else(|| format!("deny until {until} for {key}"))?;
    Ok((key, until))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_ms_never_rounds_down() {
        let t = DateTime::from_timestamp(10, 1).unwrap();
        assert_eq!(ceil_ms(t), 10_001);
        let t = DateTime::from_timestamp(10, 2_000_000).unwrap();
        assert_eq!(ceil_ms(t), 10_002);
    }

    #[test]
    fn keys_hold_only_the_issuer_hash() {
        let (index, exact) = deny_store_keys(DENY_STORE_PREFIX, "https://issuer.example");
        assert!(!index.contains("issuer.example"));
        assert!(exact.starts_with("buzz:nip-fi:deny-until:"));
        assert_ne!(
            deny_store_keys(SHADOW_DENY_STORE_PREFIX, "i").0,
            deny_store_keys(DENY_STORE_PREFIX, "i").0
        );
    }

    #[test]
    fn exact_until_round_trips() {
        let t = DateTime::from_timestamp(1_795_000_000, 123_456_789).unwrap();
        let s = format!("{}:{}", t.timestamp(), t.timestamp_subsec_nanos());
        assert_eq!(parse_until(&s), Some(t));
    }
}
