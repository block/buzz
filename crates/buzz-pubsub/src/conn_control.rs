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

use std::sync::Arc;

use buzz_core::{CommunityId, TenantContext};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::community_topics::{
    run_community_subscriber, CommunityChannelFamily, CommunityTopics, DesiredCommunities,
};
use crate::topic::BUZZ_PREFIX;

/// Tenant-local Redis pub/sub channel suffix for connection-control messages.
pub const CONN_CONTROL_SUFFIX: &str = "conn-control";

/// Log label for the connection-control subscriber.
pub(crate) const CONN_CONTROL_NAME: &str = "conn-control";

/// Redis pub/sub channel for connection-control messages under `ctx`.
pub fn conn_control_channel(ctx: &TenantContext) -> String {
    conn_control_channel_for(ctx.community())
}

/// Redis pub/sub channel for connection-control messages in `community`.
///
/// The subscriber needs this without a [`TenantContext`]: it subscribes to the
/// exact channels of the communities holding live sockets on this pod, which are
/// known only as ids.
pub fn conn_control_channel_for(community: CommunityId) -> String {
    format!("{BUZZ_PREFIX}:{community}:{CONN_CONTROL_SUFFIX}")
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

/// Subscribes to the exact `buzz:{community}:conn-control` channels for the
/// communities holding live sockets on this pod and forwards scoped commands to
/// the broadcast.
///
/// `desired` is re-read on every reconcile, so a community that gains or loses
/// its last socket converges without any explicit command. Never returns. See
/// [`crate::community_topics`] for the reconciliation contract.
pub async fn run_conn_control_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<ScopedConnControl>,
    topics: Arc<CommunityTopics>,
    desired: DesiredCommunities,
) {
    run_community_subscriber(
        redis_url,
        ConnControlFamily { broadcast_tx },
        topics,
        desired,
    )
    .await;
}

struct ConnControlFamily {
    broadcast_tx: broadcast::Sender<ScopedConnControl>,
}

impl CommunityChannelFamily for ConnControlFamily {
    fn channel(&self, community: CommunityId) -> String {
        conn_control_channel_for(community)
    }

    fn parse_channel(&self, channel: &str) -> Option<CommunityId> {
        parse_conn_control_channel(channel)
    }

    fn deliver(&self, community_id: CommunityId, payload: &str) {
        let command: ConnControl = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Failed to deserialize conn-control message: {e}");
                return;
            }
        };

        let scoped = ScopedConnControl {
            community_id,
            command,
        };

        if self.broadcast_tx.send(scoped).is_err() {
            tracing::trace!("No conn-control receivers — message dropped");
        }
    }
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
}
