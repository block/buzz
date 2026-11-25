//! Cross-pod connection-control commands over Redis pub/sub.
//!
//! Under horizontal scaling a member's live connections may land on any pod,
//! so a moderation action taken on one pod (a ban) must reach the pod holding
//! the victim's socket. This module carries connection-control intents — today
//! only "disconnect this pubkey" — to every pod, which each apply locally
//! against their own [`crate::ConnectionManager`].
//!
//! This is deliberately a **separate** channel from `cache_invalidation`: a
//! cache-key drop is a pure, idempotent hint (the DB is re-read on the next
//! access), whereas a disconnect is an imperative, non-idempotent action on a
//! live socket. Folding it into the cache-invalidation enum would break that
//! module's stated invariant ("a pure cache-key drop, never an evict payload").
//! The DB ban row remains the durable backstop: even if a disconnect message is
//! dropped, the next auth attempt is refused at the auth seam.

use buzz_core::{CommunityId, TenantContext};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::time::{Duration, Instant};
use uuid::Uuid;

use crate::topic::BUZZ_PREFIX;

/// Tenant-local Redis pub/sub channel suffix for connection-control messages.
pub const CONN_CONTROL_SUFFIX: &str = "conn-control";

/// Pattern the subscriber uses to receive connection-control messages for every
/// community this pod may hold connections for.
pub const CONN_CONTROL_PATTERN: &str = "buzz:*:conn-control";

/// Redis pub/sub channel for connection-control messages under `ctx`.
pub fn conn_control_channel(ctx: &TenantContext) -> String {
    format!("{BUZZ_PREFIX}:{}:{CONN_CONTROL_SUFFIX}", ctx.community())
}

// ── NIP-FI global disconnect channel ─────────────────────────────────────────

/// Global (issuer-scoped, not community-scoped) Redis pub/sub channel for NIP-FI
/// disconnect commands.  A single channel covers all communities because NIP-FI
/// deny entries apply across the full issuer domain — the community a user is
/// connected to at that moment is irrelevant.
pub const NIP_FI_DISCONNECT_CHANNEL: &str = "buzz:nip-fi:disconnect";

/// Channel for disconnects accepted in shadow mode.  Only shadow pods
/// subscribe, so a shadow command never reaches an enforce pod.
pub const NIP_FI_SHADOW_DISCONNECT_CHANNEL: &str = "buzz:nip-fi:shadow-disconnect";

/// A NIP-FI admin-disconnect command broadcast cross-pod after the local deny
/// entry is inserted.  Every pod merges this entry into its own deny map and
/// closes any matching sessions (same `max(until)` rule as the local path).
///
/// Transmitted asynchronously — the HTTP response does not wait on remote-pod
/// delivery; the spec's asynchronous-success semantics are preserved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NipFiDisconnect {
    /// Exact `iss` URI of the issuer that originated the command.
    pub issuer: String,
    /// 32 raw bytes of the target Nostr public key.
    pub pubkey_bytes: Vec<u8>,
    /// `until` seconds since the Unix epoch (whole-second component).
    pub until_unix: i64,
    /// Nanosecond sub-second component of `until` (0..1_000_000_000).
    /// Transmitted alongside `until_unix` so the full sub-second precision of
    /// the signed command JWT is preserved across pod boundaries.
    #[serde(default)]
    pub until_unix_nanos: u32,
}

/// State of this pod's NIP-FI disconnect subscription.  Every confirmed
/// subscribe and every loss increments `epoch`; `live` tells which it was.
/// A consumer that restores state on each live epoch misses no message
/// published after Redis confirmed the subscription.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NipFiSubscription {
    /// Increments on every change.
    pub epoch: u64,
    /// Redis has confirmed the subscription and it has not ended since.
    pub live: bool,
}

fn set_subscription(tx: &tokio::sync::watch::Sender<NipFiSubscription>, live: bool) {
    tx.send_modify(|s| {
        s.epoch += 1;
        s.live = live;
    });
}

/// Encode a [`NipFiDisconnect`] message to a JSON string for publication on
/// the Redis pub/sub channel.
///
/// This is the single publication path — the HTTP handler and any future
/// publisher must call this function rather than serialising directly, so the
/// wire format is defined in one place and the round-trip oracle can cover it.
pub fn encode_nip_fi_disconnect(message: &NipFiDisconnect) -> Result<String, serde_json::Error> {
    serde_json::to_string(message)
}

/// Decode a [`NipFiDisconnect`] message from a JSON string received from the
/// Redis pub/sub channel.
///
/// This is the single consumption path — the subscriber and any future consumer
/// must call this function rather than deserialising directly, so the wire format
/// is defined in one place and the round-trip oracle can cover it.
pub fn decode_nip_fi_disconnect(payload: &str) -> Result<NipFiDisconnect, serde_json::Error> {
    serde_json::from_str(payload)
}

/// Parse a connection-control Redis channel into its scoped community id.
pub fn parse_conn_control_channel(channel: &str) -> Option<CommunityId> {
    let mut parts = channel.split(':');
    if parts.next()? != BUZZ_PREFIX {
        return None;
    }
    let community_id = Uuid::parse_str(parts.next()?).ok()?;
    if parts.next()? != CONN_CONTROL_SUFFIX {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(CommunityId::from_uuid(community_id))
}

/// A connection-control command to apply on every pod.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op")]
pub enum ConnControl {
    /// Disconnect every live socket bound to the carrying community.
    ///
    /// Archive callers include the durable archive-transition timestamp. Each
    /// receiver checks it under a row lock before disconnecting, so a delayed
    /// command cannot disconnect a subsequently restored community. `None`
    /// (permanent deletion, or archive from a relay predating the timestamp)
    /// closes only while the row is inactive, and fails closed as a deletion
    /// when the receiver cannot evaluate that fence.
    DisconnectCommunity {
        /// Exact archive transition this command belongs to, when reversible.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        archived_at: Option<DateTime<Utc>>,
    },
    /// Disconnect every live connection authenticated as `pubkey` in the
    /// carrying community — live ban enforcement. `pubkey` is 32 raw bytes.
    /// `event_id` and `reason` reproduce the same NIP-01 `OK` frame the origin
    /// pod sent, so a member disconnected on any pod learns why.
    DisconnectPubkey {
        /// Banned member's pubkey bytes.
        pubkey: Vec<u8>,
        /// Id echoed in the closing `OK` frame (the ban event's id on origin).
        event_id: String,
        /// Human-readable close reason for the `OK` frame.
        reason: String,
        /// Close only sockets admitted with no recorded owner. Sent when an
        /// agent's owner is first recorded, so its ownerless sockets reconnect
        /// with the owner attached while owner-bound sockets stay up.
        #[serde(default)]
        unowned_only: bool,
    },
}

/// A connection-control command received from a community-scoped Redis channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedConnControl {
    /// Community whose connections the command applies to.
    pub community_id: CommunityId,
    /// The tenant-local connection-control command.
    pub command: ConnControl,
}

/// Initial reconnect backoff (1 second).
const BACKOFF_INITIAL_SECS: u64 = 1;
/// Maximum reconnect backoff (30 seconds).
const BACKOFF_MAX_SECS: u64 = 30;

/// Subscribes to `buzz:*:conn-control` and forwards scoped commands to the
/// broadcast. Mirrors [`crate::cache_invalidation::run_cache_invalidation_subscriber`]:
/// a reconnect loop with exponential backoff. Never returns.
pub async fn run_conn_control_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<ScopedConnControl>,
) {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;

    loop {
        match connect_and_subscribe(&redis_url, &broadcast_tx).await {
            Ok(()) => {
                backoff_secs = BACKOFF_INITIAL_SECS;
                tracing::warn!(
                    "Redis conn-control stream ended (clean disconnect) — reconnecting in {backoff_secs}s"
                );
            }
            Err(e) => {
                tracing::error!("Redis conn-control error: {e} — reconnecting in {backoff_secs}s");
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(BACKOFF_MAX_SECS);

        tracing::info!("Attempting to reconnect to Redis conn-control...");
    }
}

async fn connect_and_subscribe(
    redis_url: &str,
    broadcast_tx: &broadcast::Sender<ScopedConnControl>,
) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_async_pubsub().await?;

    conn.psubscribe(CONN_CONTROL_PATTERN).await?;

    tracing::info!("Redis conn-control subscriber connected — listening on {CONN_CONTROL_PATTERN}");

    let mut stream = conn.on_message();
    while let Some(msg) = stream.next().await {
        let channel = msg.get_channel_name();
        let Some(community_id) = parse_conn_control_channel(channel) else {
            tracing::warn!("Received conn-control message on unexpected channel: {channel}");
            continue;
        };

        let payload: String = match msg.get_payload() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get conn-control payload: {e}");
                continue;
            }
        };

        let command: ConnControl = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to deserialize conn-control message: {e}");
                continue;
            }
        };

        let scoped = ScopedConnControl {
            community_id,
            command,
        };

        if broadcast_tx.send(scoped).is_err() {
            tracing::trace!("No conn-control receivers — message dropped");
        }
    }

    Ok(())
}

// ── NIP-FI disconnect subscriber ──────────────────────────────────────────────

/// First NIP-FI reconnect delay.  Admission is closed while the subscription
/// is down, so this subscriber retries much sooner than the shared backoff.
const NIP_FI_BACKOFF_INITIAL: Duration = Duration::from_millis(250);
/// NIP-FI reconnect delay cap.
const NIP_FI_BACKOFF_MAX: Duration = Duration::from_secs(4);
/// A confirmed subscription that lasted this long counts as healthy: its end
/// reconnects at once and resets the backoff.  A shorter one is a failure,
/// so confirm-then-drop cycles keep backing off instead of spinning.
const NIP_FI_HEALTHY_LIVE: Duration = Duration::from_secs(1);

/// Subscribes to `channels` and forwards commands to the
/// broadcast, reporting each confirmed subscription and loss on
/// `subscription`.  Reconnects with a short capped backoff (see
/// [`NIP_FI_BACKOFF_MAX`]).  Never returns.
pub async fn run_nip_fi_disconnect_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<NipFiDisconnect>,
    subscription: tokio::sync::watch::Sender<NipFiSubscription>,
    channels: &'static [&'static str],
) {
    nip_fi_retry_loop(subscription, |confirmed| {
        connect_and_subscribe_nip_fi(redis_url.clone(), broadcast_tx.clone(), confirmed, channels)
    })
    .await
}

/// Reports a confirmed NIP-FI subscription: records when Redis confirmed it
/// and publishes the live epoch.
struct NipFiConfirm {
    at: Arc<Mutex<Option<Instant>>>,
    subscription: Arc<tokio::sync::watch::Sender<NipFiSubscription>>,
}

impl NipFiConfirm {
    fn confirmed(&self) {
        *self.at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        set_subscription(&self.subscription, true);
    }
}

/// The NIP-FI reconnect loop.  `attempt` runs one connection; it calls
/// [`NipFiConfirm::confirmed`] once Redis has confirmed the subscription,
/// then returns when the subscription ends.
async fn nip_fi_retry_loop<F, Fut>(
    subscription: tokio::sync::watch::Sender<NipFiSubscription>,
    mut attempt: F,
) -> !
where
    F: FnMut(NipFiConfirm) -> Fut,
    Fut: std::future::Future<Output = Result<(), redis::RedisError>>,
{
    let subscription = Arc::new(subscription);
    let mut backoff = NIP_FI_BACKOFF_INITIAL;
    loop {
        let at = Arc::new(Mutex::new(None));
        let result = attempt(NipFiConfirm {
            at: Arc::clone(&at),
            subscription: Arc::clone(&subscription),
        })
        .await;
        set_subscription(&subscription, false);
        match &result {
            Ok(()) => tracing::warn!("Redis NIP-FI disconnect stream ended (clean disconnect)"),
            Err(e) => tracing::error!("Redis NIP-FI disconnect error: {e}"),
        }

        let confirmed_at = *at.lock().unwrap_or_else(|e| e.into_inner());
        if confirmed_at.is_some_and(|at| at.elapsed() >= NIP_FI_HEALTHY_LIVE) {
            backoff = NIP_FI_BACKOFF_INITIAL;
            tracing::info!("Redis NIP-FI disconnect subscriber reconnecting now");
            continue;
        }
        tracing::info!("Redis NIP-FI disconnect subscriber reconnecting in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(NIP_FI_BACKOFF_MAX);
    }
}

async fn connect_and_subscribe_nip_fi(
    redis_url: String,
    broadcast_tx: broadcast::Sender<NipFiDisconnect>,
    confirm: NipFiConfirm,
    channels: &[&str],
) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_async_pubsub().await?;

    // Returns once Redis has confirmed the subscription.
    conn.subscribe(channels).await?;
    confirm.confirmed();

    tracing::info!("Redis NIP-FI disconnect subscriber connected — listening on {channels:?}");

    let mut stream = conn.on_message();
    while let Some(msg) = stream.next().await {
        let payload: String = match msg.get_payload() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get NIP-FI disconnect payload: {e}");
                continue;
            }
        };

        let command: NipFiDisconnect = match decode_nip_fi_disconnect(&payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to deserialize NIP-FI disconnect message: {e}");
                continue;
            }
        };

        if broadcast_tx.send(command).is_err() {
            tracing::trace!("No NIP-FI disconnect receivers — message dropped");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One scripted NIP-FI connection attempt: `None` fails before Redis
    /// confirms; `Some(d)` confirms, stays live for `d`, then drops.
    async fn run_script(script: Vec<Option<Duration>>) -> (Vec<Duration>, Vec<NipFiSubscription>) {
        let (tx, mut rx) = tokio::sync::watch::channel(NipFiSubscription::default());
        let starts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let origin = Instant::now();
        let steps = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(
            script,
        )));
        let recorded = std::sync::Arc::clone(&starts);
        let task = tokio::spawn(async move {
            nip_fi_retry_loop(tx, |confirm| {
                recorded.lock().unwrap().push(origin.elapsed());
                let step = steps.lock().unwrap().pop_front();
                async move {
                    match step {
                        None => std::future::pending().await,
                        Some(None) => {
                            Err(redis::RedisError::from(std::io::Error::other("refused")))
                        }
                        Some(Some(live)) => {
                            confirm.confirmed();
                            tokio::time::sleep(live).await;
                            Err(redis::RedisError::from(std::io::Error::other("dropped")))
                        }
                    }
                }
            })
            .await
        });
        let mut states = Vec::new();
        while let Ok(Ok(())) = tokio::time::timeout(Duration::from_secs(60), rx.changed()).await {
            states.push(*rx.borrow_and_update());
        }
        task.abort();
        let starts = starts.lock().unwrap().clone();
        (starts, states)
    }

    fn gaps(starts: &[Duration]) -> Vec<Duration> {
        starts.windows(2).map(|w| w[1] - w[0]).collect()
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[tokio::test(start_paused = true)]
    async fn nip_fi_failed_connects_back_off_to_a_four_second_cap() {
        let (starts, states) = run_script(vec![None; 7]).await;
        assert_eq!(
            gaps(&starts),
            [250, 500, 1000, 2000, 4000, 4000, 4000].map(MS),
        );
        assert!(
            states.iter().all(|s| !s.live),
            "never confirmed: {states:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn nip_fi_healthy_subscription_end_reconnects_at_once_and_resets() {
        // Climb the backoff, then a healthy 5 s subscription, then failures.
        let (starts, states) = run_script(vec![
            None,
            None,
            None,
            Some(Duration::from_secs(5)),
            None,
            None,
        ])
        .await;
        assert_eq!(
            gaps(&starts),
            [250, 500, 1000, 5000, 250, 500].map(MS),
            "healthy end retries immediately, then the backoff restarts at 250 ms"
        );
        // One live epoch, then its loss; every attempt bumps the epoch
        // (6 losses + 1 confirmation), and the watch ends not live.
        let live: Vec<_> = states.iter().filter(|s| s.live).collect();
        assert_eq!(live.len(), 1);
        let last = states.last().expect("states");
        assert_eq!((last.epoch, last.live), (7, false));
    }

    #[tokio::test(start_paused = true)]
    async fn nip_fi_confirm_then_drop_keeps_backing_off() {
        let flap = Some(MS(100));
        let (starts, states) = run_script(vec![flap; 7]).await;
        // Each gap is 100 ms live plus the backoff step: no reset, no spin.
        assert_eq!(
            gaps(&starts),
            [350, 600, 1100, 2100, 4100, 4100, 4100].map(MS),
        );
        // Every flap reports confirmation then loss, in order.
        assert_eq!(states.len(), 14);
        for (i, s) in states.iter().enumerate() {
            assert_eq!(s.epoch, i as u64 + 1);
            assert_eq!(s.live, i % 2 == 0);
        }
    }

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[test]
    fn conn_control_channel_is_community_scoped() {
        let a = ctx(0xaaaa, "a.example");
        let b = ctx(0xbbbb, "b.example");
        assert_eq!(
            conn_control_channel(&a),
            format!("buzz:{}:conn-control", a.community())
        );
        assert_ne!(conn_control_channel(&a), conn_control_channel(&b));
    }

    #[test]
    fn parse_round_trips_the_community() {
        let a = ctx(0x1234, "a.example");
        let channel = conn_control_channel(&a);
        assert_eq!(parse_conn_control_channel(&channel), Some(a.community()));
    }

    #[test]
    fn parse_rejects_foreign_channels() {
        assert_eq!(
            parse_conn_control_channel("buzz:not-a-uuid:conn-control"),
            None
        );
        assert_eq!(parse_conn_control_channel("buzz:*:cache-invalidate"), None);
        let a = ctx(0x1234, "a.example");
        let extended = format!("{}:extra", conn_control_channel(&a));
        assert_eq!(parse_conn_control_channel(&extended), None);
    }

    #[test]
    fn disconnect_community_command_serde_round_trips() {
        let cmd = ConnControl::DisconnectCommunity { archived_at: None };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"op":"DisconnectCommunity"}"#);
        assert_eq!(serde_json::from_str::<ConnControl>(&json).unwrap(), cmd);
    }

    #[test]
    fn archived_community_disconnect_carries_the_lifecycle_fence() {
        let archived_at = chrono::DateTime::parse_from_rfc3339("2026-08-31T12:34:56.123456Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let cmd = ConnControl::DisconnectCommunity {
            archived_at: Some(archived_at),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""archived_at":"2026-08-31T12:34:56.123456Z""#));
        assert_eq!(serde_json::from_str::<ConnControl>(&json).unwrap(), cmd);
    }

    #[test]
    fn legacy_consumer_accepts_archived_community_disconnect_payload() {
        #[derive(Debug, Deserialize, PartialEq, Eq)]
        #[serde(tag = "op")]
        enum LegacyConnControl {
            DisconnectCommunity,
        }

        let archived_at = chrono::DateTime::parse_from_rfc3339("2026-08-31T12:34:56.123456Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let json = serde_json::to_string(&ConnControl::DisconnectCommunity {
            archived_at: Some(archived_at),
        })
        .unwrap();

        assert_eq!(
            serde_json::from_str::<LegacyConnControl>(&json).unwrap(),
            LegacyConnControl::DisconnectCommunity
        );
    }

    #[test]
    fn unknown_command_is_rejected_without_affecting_later_messages() {
        assert!(serde_json::from_str::<ConnControl>(r#"{"op":"FutureCommand"}"#).is_err());
        let known =
            serde_json::to_string(&ConnControl::DisconnectCommunity { archived_at: None }).unwrap();
        assert_eq!(
            serde_json::from_str::<ConnControl>(&known).unwrap(),
            ConnControl::DisconnectCommunity { archived_at: None }
        );
    }

    #[test]
    fn disconnect_command_serde_round_trips() {
        let cmd = ConnControl::DisconnectPubkey {
            pubkey: vec![7u8; 32],
            event_id: "abc123".to_string(),
            reason: "blocked: you are banned from this community".to_string(),
            unowned_only: true,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(serde_json::from_str::<ConnControl>(&json).unwrap(), cmd);
    }

    // ── NipFiDisconnect serde ─────────────────────────────────────────────────

    #[test]
    fn nip_fi_disconnect_serde_round_trips() {
        let cmd = NipFiDisconnect {
            issuer: "https://idp.example.com".to_string(),
            pubkey_bytes: vec![0xabu8; 32],
            until_unix: 9_999_999_999,
            until_unix_nanos: 500_000_000,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let decoded: NipFiDisconnect = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, cmd);
    }

    #[test]
    fn nip_fi_disconnect_nanos_default_to_zero_when_absent() {
        // Old messages (without the until_unix_nanos field) must still deserialize.
        let legacy_json = r#"{"issuer":"https://idp.example.com","pubkey_bytes":[171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171,171],"until_unix":9999999999}"#;
        let decoded: NipFiDisconnect = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(
            decoded.until_unix_nanos, 0,
            "missing nanos field must default to 0"
        );
        assert_eq!(decoded.until_unix, 9_999_999_999);
    }

    #[test]
    fn nip_fi_disconnect_channel_is_global_not_community_scoped() {
        // Must NOT contain a community UUID segment — it's issuer-global.
        assert_eq!(NIP_FI_DISCONNECT_CHANNEL, "buzz:nip-fi:disconnect");
        assert!(!NIP_FI_DISCONNECT_CHANNEL.contains("conn-control"));
    }

    #[test]
    fn nip_fi_disconnect_malformed_payload_is_skipped() {
        // Simulate what the subscriber does: malformed JSON produces an error
        // and the message is skipped (no panic).
        let malformed = r#"{"issuer": 42}"#; // wrong type for issuer
        assert!(serde_json::from_str::<NipFiDisconnect>(malformed).is_err());
        // Well-formed payload still parses.
        let good = serde_json::to_string(&NipFiDisconnect {
            issuer: "https://a.example.com".to_string(),
            pubkey_bytes: vec![1u8; 32],
            until_unix: 1_000_000,
            until_unix_nanos: 0,
        })
        .unwrap();
        assert!(serde_json::from_str::<NipFiDisconnect>(&good).is_ok());
    }
}
