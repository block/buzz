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
//! * `{deny}-revoke-through:{sha256(iss)}` — hash, field `pubkey_hex`, value
//!   the merged `revoke_through` cutoff, present only while the entry is
//!   scoped to it; a missing field is a whole-key deny.  It lives outside the
//!   exact-`until` hash so relays that predate cutoffs still decode that hash
//!   and read the entry as a whole-key deny; those relays clear the field on
//!   every write, so their full revokes are never narrowed.
//!
//! Only the issuer's hash is stored; readers map it back through configured
//! issuers.

use buzz_auth::nip_fi::{
    namespaced_command_replay_key, COMMAND_REPLAY_PREFIX, SHADOW_COMMAND_REPLAY_PREFIX,
};
use buzz_auth::{
    error::AuthError, ClaimOutcome, CommandClaim, CommandReplayGuard, DenyEntry, DenyScope,
    LoadDeniesError, DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS,
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
/// capacity, revoke_through ('' for the whole key).  Whether the command and
/// the stored entry are still active is judged by Redis `TIME` as the script
/// runs, not by the clock the command was authenticated under.  A stored
/// cutoff restore would reject returns 'unreadable' before any write.
const ACCEPT_SCRIPT: &str = r"
if redis.call('EXISTS', KEYS[1]) == 1 then return 'replay' end
-- A stored cutoff restore would reject is refused here, before any write.
local stored = redis.call('HGET', KEYS[4], ARGV[2])
if stored then
  local c = string.match(stored, '^[+-]?%d+$') and tonumber(stored)
  if not c or c < -8334601228800 or c > 8210266876799 then return 'unreadable' end
end
local t = redis.call('TIME')
local ts, tn = tonumber(t[1]), tonumber(t[2]) * 1000
local now_ms = string.format('%.0f', ts * 1000 + math.floor(tn / 1000000))
local function live(v)
  local s, n = string.match(v, '^(%-?%d+):(%d+)$')
  s, n = tonumber(s), tonumber(n)
  return s > ts or (s == ts and n > tn)
end
local active = live(ARGV[4] .. ':' .. ARGV[5])
if active then
  local expired = redis.call('ZRANGEBYSCORE', KEYS[2], '-inf', '(' .. now_ms)
  for _, member in ipairs(expired) do
    redis.call('HDEL', KEYS[3], member)
    redis.call('HDEL', KEYS[4], member)
  end
  redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', '(' .. now_ms)
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
  -- Scope merge: an active stored entry without a cutoff is the whole key and
  -- wins; two cutoffs keep the later one.  An expired entry contributes none.
  local cut = ARGV[7]
  if cut ~= '' and cur and live(cur) then
    local prior = redis.call('HGET', KEYS[4], ARGV[2])
    if not prior or not tonumber(prior) then
      cut = ''
    elseif tonumber(prior) > tonumber(cut) then
      cut = prior
    end
  end
  if cut == '' then
    redis.call('HDEL', KEYS[4], ARGV[2])
  else
    redis.call('HSET', KEYS[4], ARGV[2], cut)
  end
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
                .arg(
                    claim
                        .scope
                        .revoke_through()
                        .map_or_else(String::new, |c| c.to_string()),
                )
                .invoke_async(&mut conn)
                .await
                .map_err(internal)?;
            match reply.as_str() {
                "claimed" => Ok(ClaimOutcome::Claimed),
                "replay" => Ok(ClaimOutcome::Replay),
                "full" => Ok(ClaimOutcome::Full),
                "unreadable" => {
                    let record = format!("revoke_through for {}", claim.target.to_hex());
                    tracing::warn!(record, "nip-fi command store: unreadable deny record");
                    Err(internal(format!("unreadable deny record: {record}")))
                }
                other => Err(internal(format!("unexpected reply: {other}"))),
            }
        })
    }

    fn load_denies<'a>(&'a self, issuer: &'a str, now: DateTime<Utc>) -> buzz_auth::LoadDenies<'a> {
        Box::pin(async move {
            let (_, exact) = deny_store_keys(&self.deny_prefix, issuer);
            let mut conn = self.pool.clone();
            // One script, so a cutoff is never paired with a deadline it was
            // not written with.
            let (entries, cutoffs): (RawHash, RawHash) = redis::Script::new(SNAPSHOT_SCRIPT)
                .key(exact)
                .key(revoke_through_key(&self.deny_prefix, issuer))
                .invoke_async(&mut conn)
                .await
                .map_err(internal)?;
            // An unreadable record fails the load: skipping it would reopen
            // admission without that revocation.  Every other record is
            // still decoded, so the caller can apply them meanwhile.
            let mut bad = None;
            let entries = utf8_pairs(entries, &mut bad);
            let cutoffs = utf8_pairs(cutoffs, &mut bad);
            let untils = decode_untils(&entries, now, &mut bad);
            let denies = join_cutoffs(untils, &entries, cutoffs, &mut bad);
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

/// A hash as `HGETALL` returns it: field and value bytes.
type RawHash = Vec<(Vec<u8>, Vec<u8>)>;

/// Keeps the first unreadable record and logs every one.
fn note(bad: &mut Option<String>, record: String) {
    tracing::warn!(record, "nip-fi command store: unreadable deny record");
    bad.get_or_insert(record);
}

/// A hash's fields and values as text; a record that isn't UTF-8 is
/// unreadable and left out.
fn utf8_pairs(raw: RawHash, bad: &mut Option<String>) -> Vec<(String, String)> {
    raw.into_iter()
        .filter_map(
            |(key, value)| match (String::from_utf8(key), String::from_utf8(value)) {
                (Ok(key), Ok(value)) => Some((key, value)),
                _ => {
                    note(bad, "deny record that is not UTF-8".into());
                    None
                }
            },
        )
        .collect()
}

/// KEYS: exact, cutoff.  Both hashes as one snapshot.
const SNAPSHOT_SCRIPT: &str = r"
return {redis.call('HGETALL', KEYS[1]), redis.call('HGETALL', KEYS[2])}
";

/// The exact-`until` hash's active entries.  This is the whole of the reader
/// in relays that predate cutoffs, which therefore read every entry as a
/// whole-key deny.  An unreadable record is noted in `bad` and left out; the
/// others still decode.
fn decode_untils(
    entries: &[(String, String)],
    now: DateTime<Utc>,
    bad: &mut Option<String>,
) -> Vec<(PublicKey, DateTime<Utc>)> {
    let mut denies = Vec::with_capacity(entries.len());
    for (key, until) in entries {
        let Ok(pubkey) = PublicKey::from_hex(key) else {
            note(bad, format!("deny key {key}"));
            continue;
        };
        match parse_until(until) {
            Some(until) if until > now => denies.push((pubkey, until)),
            Some(_) => {}
            None => note(bad, format!("deny until {until} for {key}")),
        }
    }
    denies
}

/// Scope each active deny by its cutoff field; a missing field is the whole
/// key.  A cutoff that doesn't decode, or that names no stored deny, is
/// noted in `bad`; a deny under an undecodable cutoff stays the whole key.
fn join_cutoffs(
    untils: Vec<(PublicKey, DateTime<Utc>)>,
    entries: &[(String, String)],
    cutoffs: Vec<(String, String)>,
    bad: &mut Option<String>,
) -> Vec<(PublicKey, DenyEntry)> {
    let denied: std::collections::HashSet<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    let mut scopes = std::collections::HashMap::with_capacity(cutoffs.len());
    for (key, cutoff) in cutoffs {
        if !denied.contains(key.as_str()) {
            note(bad, format!("revoke_through for {key} without a deny"));
            continue;
        }
        match cutoff
            .parse::<i64>()
            .ok()
            .filter(|c| DateTime::from_timestamp(*c, 0).is_some())
        {
            Some(c) => {
                scopes.insert(key, DenyScope::IssuedThrough(c));
            }
            None => note(bad, format!("revoke_through {cutoff} for {key}")),
        }
    }
    untils
        .into_iter()
        .map(|(key, until)| {
            let scope = scopes
                .get(&key.to_hex())
                .copied()
                .unwrap_or(DenyScope::WholeKey);
            (key, DenyEntry { until, scope })
        })
        .collect()
}

fn parse_until(value: &str) -> Option<DateTime<Utc>> {
    let (secs, nanos) = value.split_once(':')?;
    DateTime::from_timestamp(secs.parse().ok()?, nanos.parse().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The accept script's cutoff bounds are restore's `from_timestamp` range.
    #[test]
    fn accept_cutoff_bounds_match_restore() {
        for bound in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
            assert!(ACCEPT_SCRIPT.contains(&bound.timestamp().to_string()));
        }
    }

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

    /// A bad cutoff does not hide the others: a good one still scopes its
    /// key, a malformed one leaves its deny the whole key, an orphan adds
    /// nothing, and the first bad record is kept.
    #[test]
    fn cutoff_join_keeps_readable_records_past_bad_ones() {
        let [good, malformed, orphan] =
            std::array::from_fn(|i| PublicKey::from_hex(&format!("{:064x}", i + 1)).unwrap());
        let until = DateTime::from_timestamp(100, 0).unwrap();
        let entries: Vec<_> = [good, malformed]
            .iter()
            .map(|k| (k.to_hex(), "100:0".to_owned()))
            .collect();
        let cutoffs = vec![
            (good.to_hex(), "10".to_owned()),
            (malformed.to_hex(), "12.5".to_owned()),
            (orphan.to_hex(), "10".to_owned()),
        ];
        let mut bad = None;
        let joined = join_cutoffs(
            vec![(good, until), (malformed, until)],
            &entries,
            cutoffs,
            &mut bad,
        );
        assert_eq!(
            joined,
            vec![
                (
                    good,
                    DenyEntry {
                        until,
                        scope: DenyScope::IssuedThrough(10)
                    }
                ),
                (
                    malformed,
                    DenyEntry {
                        until,
                        scope: DenyScope::WholeKey
                    }
                ),
            ]
        );
        assert_eq!(
            bad,
            Some(format!("revoke_through 12.5 for {}", malformed.to_hex()))
        );
    }

    /// A full issuer at the default capacity joins every cutoff to its key.
    #[test]
    fn cutoff_join_handles_a_full_issuer() {
        let entries: Vec<_> = (0..50_000)
            .map(|i| (format!("{i:064x}"), "1:0".to_owned()))
            .collect();
        let cutoffs = entries
            .iter()
            .map(|(k, _)| (k.clone(), "10".to_owned()))
            .collect();
        let mut bad = None;
        join_cutoffs(Vec::new(), &entries, cutoffs, &mut bad);
        assert_eq!(bad, None);
    }

    #[test]
    fn exact_until_round_trips() {
        let t = DateTime::from_timestamp(1_795_000_000, 123_456_789).unwrap();
        let s = format!("{}:{}", t.timestamp(), t.timestamp_subsec_nanos());
        assert_eq!(parse_until(&s), Some(t));
    }
}
