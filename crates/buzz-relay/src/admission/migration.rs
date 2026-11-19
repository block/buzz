//! A deployment-wide budget for signed events labeled as migration writes.

use buzz_pubsub::RedisConnection;
use nostr::Event;
use redis::{RedisResult, Script};

const KEY: &str = "buzz:ratelimit:migration_writes";
const DEFAULT_EVENTS_PER_MINUTE: u32 = 12;
pub(crate) const MAX_EVENTS_PER_MINUTE: u32 = 60_000;
const RETRY_AFTER_SECS: u64 = 5;

// Read the live limit and grant admission in one Redis operation. Use Redis
// time so callers on different pods spend the same budget. Idle time makes
// one permit available and never accumulates a burst.
const ADMIT_SCRIPT: &str = r#"
local rate = redis.call('HGET', KEYS[1], 'events_per_minute')
if rate == false then rate = ARGV[1] end
if not string.match(rate, '^%d+$') or tonumber(rate) > tonumber(ARGV[2]) then
    return redis.error_reply('invalid migration write limit')
end
rate = tonumber(rate)
if rate == 0 then return -1 end

local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000000 + tonumber(time[2])
local last = redis.call('HGET', KEYS[1], 'last_admitted_us')
if last then
    if not string.match(last, '^%d+$') or tonumber(last) > 9007199254740991 then
        return redis.error_reply('invalid migration admission timestamp')
    end
    local remaining = math.ceil(60000000 / rate) - (now - tonumber(last))
    if remaining > 0 then return math.min(60, math.ceil(remaining / 1000000)) end
end
redis.call('HSET', KEYS[1], 'last_admitted_us', string.format('%.0f', now))
return 0
"#;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Throttled(u64),
    Paused,
    Unavailable,
    InvalidEvent(String),
}

impl Refusal {
    pub(crate) fn retry_after(&self) -> Option<u64> {
        match self {
            Self::Throttled(seconds) => Some(*seconds),
            Self::Paused | Self::Unavailable => Some(RETRY_AFTER_SECS),
            Self::InvalidEvent(_) => None,
        }
    }

    fn reason(&self) -> &'static str {
        match self {
            Self::Throttled(_) => "quota",
            Self::Paused => "paused",
            Self::Unavailable => "unavailable",
            Self::InvalidEvent(_) => "invalid",
        }
    }

    pub(crate) fn message(&self) -> String {
        // Desktop classifies publish delivery from these exact wire messages.
        match self {
            Self::Throttled(seconds) => quota_message(*seconds),
            Self::Paused => quota_message(RETRY_AFTER_SECS),
            Self::Unavailable => "rate-limited: shared admission unavailable".to_owned(),
            Self::InvalidEvent(reason) => format!("invalid: {reason}"),
        }
    }
}

fn quota_message(seconds: u64) -> String {
    format!("rate-limited: quota exceeded; retry in {seconds}s")
}

pub(crate) async fn read_limit(redis: &RedisConnection) -> RedisResult<u32> {
    let rate: Option<String> = redis::cmd("HGET")
        .arg(KEY)
        .arg("events_per_minute")
        .query_async(&mut redis.clone())
        .await?;
    match rate {
        None => Ok(DEFAULT_EVENTS_PER_MINUTE),
        Some(rate) => rate
            .parse::<u32>()
            .ok()
            .filter(|value| {
                rate.bytes().all(|b| b.is_ascii_digit()) && *value <= MAX_EVENTS_PER_MINUTE
            })
            .ok_or_else(|| {
                redis::RedisError::from((
                    redis::ErrorKind::UnexpectedReturnType,
                    "invalid migration write limit",
                ))
            }),
    }
}

pub(crate) async fn set_limit(redis: &RedisConnection, events_per_minute: u32) -> RedisResult<()> {
    redis::cmd("HSET")
        .arg(KEY)
        .arg("events_per_minute")
        .arg(events_per_minute)
        .query_async(&mut redis.clone())
        .await
}

fn is_labeled(event: &Event) -> bool {
    event.tags.iter().any(
        |tag| matches!(tag.as_slice(), [name, source] if name == "migration" && !source.is_empty()),
    )
}

pub(crate) async fn check_event(
    redis: &RedisConnection,
    event: &Event,
    transport: &'static str,
) -> Result<(), Refusal> {
    if !is_labeled(event) {
        return Ok(());
    }

    // Verify the signed label before spending the fleet-wide permit. Accepted
    // events still take the regular validation and authorization path.
    let candidate = event.clone();
    match tokio::task::spawn_blocking(move || buzz_core::verification::verify_event(&candidate))
        .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(Refusal::InvalidEvent(error.to_string())),
        Err(error) => {
            tracing::error!(%error, "migration event verification task failed");
            return Err(Refusal::Unavailable);
        }
    }

    let result = Script::new(ADMIT_SCRIPT)
        .key(KEY)
        .arg(DEFAULT_EVENTS_PER_MINUTE)
        .arg(MAX_EVENTS_PER_MINUTE)
        .invoke_async::<i64>(&mut redis.clone())
        .await;
    let refusal = match result {
        Ok(0) => return Ok(()),
        Ok(-1) => Refusal::Paused,
        Ok(seconds) if seconds > 0 => Refusal::Throttled(seconds as u64),
        result => {
            tracing::warn!(?result, "migration write admission unavailable");
            Refusal::Unavailable
        }
    };
    metrics::counter!(
        "buzz_admission_rejections_total",
        "transport" => transport,
        "reason" => refusal.reason(),
        "bucket" => "migration_writes",
    )
    .increment(1);
    Err(refusal)
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn classifies_only_exact_migration_tags_with_a_source() {
        let keys = Keys::generate();
        for (tag, expected) in [
            (vec!["migration", "slack"], true),
            (vec!["migration", "example-import"], true),
            (vec!["migration", "slack", "extra"], false),
            (vec!["migration", ""], false),
            (vec!["migration"], false),
            (vec!["Migration", "slack"], false),
            (vec!["t", "migration"], false),
        ] {
            let event = EventBuilder::new(Kind::TextNote, "")
                .tag(Tag::parse(tag.clone()).unwrap())
                .sign_with_keys(&keys)
                .unwrap();
            assert_eq!(super::is_labeled(&event), expected, "{tag:?}");
        }
    }
}
