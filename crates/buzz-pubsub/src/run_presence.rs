//! Atomic per-run leases, shared across relay nodes. Disconnect never erases peers.
use crate::error::PubSubError;
use buzz_core::{run_presence::RunPresence, TenantContext};
use deadpool_redis::Pool;
use nostr::PublicKey;

fn key(ctx: &TenantContext, author: &PublicKey) -> String {
    format!("buzz:{}:presence-runs:{}", ctx.community(), author.to_hex())
}
// Bound active runs AND ordering tombstones. Retain offline fences beyond the
// maximum admissible timestamp window, without a process-local connection count.
const UPDATE: &str = r#"
local values = redis.call('HGETALL', KEYS[1])
local now = tonumber(ARGV[3])
for i = 1, #values, 2 do
  local value = cjson.decode(values[i+1])
  if value.expires_at + 180 <= now then redis.call('HDEL', KEYS[1], values[i]) end
end
local old = redis.call('HGET', KEYS[1], ARGV[1])
local incoming = cjson.decode(ARGV[2])
if old then
  old = cjson.decode(old)
  if old.seq >= incoming.seq or old.status == 'offline' then return 0 end
elseif redis.call('HLEN', KEYS[1]) >= 32 then
  return -1
end
redis.call('HSET', KEYS[1], ARGV[1], ARGV[2])
redis.call('EXPIRE', KEYS[1], 360)
return 1
"#;

/// Atomically accept a newer pulse; duplicates/reordered pulses do not renew TTL.
/// Returns false for obsolete pulses; saturation is a visible error, not eviction.
pub async fn update(
    pool: &Pool,
    ctx: &TenantContext,
    author: &PublicKey,
    pulse: &RunPresence,
    now: u64,
) -> Result<bool, PubSubError> {
    let mut conn = pool.get().await?;
    let result: i64 = redis::Script::new(UPDATE)
        .key(key(ctx, author))
        .arg(&pulse.run)
        .arg(serde_json::to_string(pulse)?)
        .arg(now)
        .invoke_async(&mut conn)
        .await?;
    if result < 0 {
        return Err(PubSubError::PresenceRunLimit);
    }
    Ok(result == 1)
}
/// Read live leases with their original deadlines. Errors are not empty/offline.
pub async fn active(
    pool: &Pool,
    ctx: &TenantContext,
    author: &PublicKey,
    now: u64,
) -> Result<Vec<RunPresence>, PubSubError> {
    let mut conn = pool.get().await?;
    let values: Vec<String> = redis::cmd("HVALS")
        .arg(key(ctx, author))
        .query_async(&mut conn)
        .await?;
    let mut runs = Vec::new();
    for value in values {
        let run: RunPresence = serde_json::from_str(&value)?;
        if run.status != "offline" && run.expires_at > now {
            runs.push(run);
        }
    }
    runs.sort_by(|a, b| a.run.cmp(&b.run));
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::{run_presence::Location, CommunityId};
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn parallel_runs_ordering_offline_and_tenant_isolation() {
        let pool = crate::test_util::make_test_pool();
        let ctx =
            TenantContext::resolved(CommunityId::from_uuid(uuid::Uuid::new_v4()), "test.example");
        let other = TenantContext::resolved(
            CommunityId::from_uuid(uuid::Uuid::new_v4()),
            "other.example",
        );
        let author = nostr::Keys::generate().public_key();
        let first = RunPresence {
            run: "a".repeat(32),
            seq: 0,
            status: "online".into(),
            expires_at: 280,
            location: Some(Location {
                host: author.to_hex(),
                label: "One".into(),
            }),
            registration: None,
        };
        let mut second = first.clone();
        second.run = "b".repeat(32);
        assert!(update(&pool, &ctx, &author, &first, 100).await.unwrap());
        assert!(update(&pool, &ctx, &author, &second, 100).await.unwrap());
        assert!(!update(&pool, &ctx, &author, &first, 120).await.unwrap());
        assert_eq!(active(&pool, &ctx, &author, 120).await.unwrap().len(), 2);
        assert!(active(&pool, &other, &author, 120)
            .await
            .unwrap()
            .is_empty());
        let mut stop = first.clone();
        stop.seq = 1;
        stop.status = "offline".into();
        assert!(update(&pool, &ctx, &author, &stop, 130).await.unwrap());
        assert!(!update(&pool, &ctx, &author, &first, 140).await.unwrap());
        let mut late = first.clone();
        late.seq = 2;
        assert!(!update(&pool, &ctx, &author, &late, 150).await.unwrap());
        assert_eq!(
            active(&pool, &ctx, &author, 150).await.unwrap(),
            vec![second]
        );
        assert!(active(&pool, &ctx, &author, 280).await.unwrap().is_empty());
    }
}
