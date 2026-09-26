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
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
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
    DisconnectCommunity,
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

/// Subscribes to [`NIP_FI_DISCONNECT_CHANNEL`] and forwards commands to the
/// broadcast.  Mirrors [`run_conn_control_subscriber`]: reconnect loop with
/// exponential backoff.  Never returns.
pub async fn run_nip_fi_disconnect_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<NipFiDisconnect>,
) {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;

    loop {
        match connect_and_subscribe_nip_fi(&redis_url, &broadcast_tx).await {
            Ok(()) => {
                backoff_secs = BACKOFF_INITIAL_SECS;
                tracing::warn!(
                    "Redis NIP-FI disconnect stream ended (clean disconnect) — reconnecting in {backoff_secs}s"
                );
            }
            Err(e) => {
                tracing::error!(
                    "Redis NIP-FI disconnect error: {e} — reconnecting in {backoff_secs}s"
                );
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(BACKOFF_MAX_SECS);
    }
}

async fn connect_and_subscribe_nip_fi(
    redis_url: &str,
    broadcast_tx: &broadcast::Sender<NipFiDisconnect>,
) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_async_pubsub().await?;

    conn.subscribe(NIP_FI_DISCONNECT_CHANNEL).await?;

    tracing::info!(
        "Redis NIP-FI disconnect subscriber connected — listening on {NIP_FI_DISCONNECT_CHANNEL}"
    );

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
        let cmd = ConnControl::DisconnectCommunity;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(serde_json::from_str::<ConnControl>(&json).unwrap(), cmd);
    }

    #[test]
    fn unknown_command_is_rejected_without_affecting_later_messages() {
        assert!(serde_json::from_str::<ConnControl>(r#"{"op":"FutureCommand"}"#).is_err());
        let known = serde_json::to_string(&ConnControl::DisconnectCommunity).unwrap();
        assert_eq!(
            serde_json::from_str::<ConnControl>(&known).unwrap(),
            ConnControl::DisconnectCommunity
        );
    }

    #[test]
    fn disconnect_command_serde_round_trips() {
        let cmd = ConnControl::DisconnectPubkey {
            pubkey: vec![7u8; 32],
            event_id: "abc123".to_string(),
            reason: "blocked: you are banned from this community".to_string(),
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
