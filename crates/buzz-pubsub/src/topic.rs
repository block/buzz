//! Community-scoped Redis event topics.
//!
//! Pub/sub topics are a routing/performance boundary, not an authorization
//! boundary. Tenant identity still comes from [`TenantContext`] on publish /
//! retain paths, and the relay re-checks access before local fan-out.

use buzz_core::{CommunityId, TenantContext};
use uuid::Uuid;

use crate::error::PubSubError;

/// Redis key prefix for Buzz-scoped pub/sub topics and keys.
pub const BUZZ_PREFIX: &str = "buzz";

/// A tenant-local event routing scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventTopic {
    /// Events for one exact channel id.
    Channel(Uuid),
    /// Community-global events that are not exact-channel routed.
    Global,
}

/// A fully qualified event topic, including its server-resolved community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventTopicKey {
    /// Server-resolved community id.
    pub community_id: CommunityId,
    /// Tenant-local routing scope.
    pub topic: EventTopic,
}

impl EventTopicKey {
    /// Build a topic key from a resolved tenant context.
    pub fn from_context(ctx: &TenantContext, topic: EventTopic) -> Self {
        Self {
            community_id: ctx.community(),
            topic,
        }
    }

    /// Redis pub/sub channel name for this topic.
    pub fn redis_channel(&self) -> String {
        match self.topic {
            EventTopic::Channel(channel_id) => {
                format!("{BUZZ_PREFIX}:{}:channel:{channel_id}", self.community_id)
            }
            EventTopic::Global => format!("{BUZZ_PREFIX}:{}:global", self.community_id),
        }
    }

    /// Parse a Redis pub/sub channel name into a scoped event topic.
    pub fn parse_redis_channel(channel: &str) -> Result<Self, PubSubError> {
        let mut parts = channel.split(':');
        let Some(prefix) = parts.next() else {
            return Err(PubSubError::InvalidChannelKey(channel.to_string()));
        };
        if prefix != BUZZ_PREFIX {
            return Err(PubSubError::InvalidChannelKey(channel.to_string()));
        }

        let Some(community) = parts.next() else {
            return Err(PubSubError::InvalidChannelKey(channel.to_string()));
        };
        let community_id = Uuid::parse_str(community)
            .map(CommunityId::from_uuid)
            .map_err(|_| PubSubError::InvalidChannelKey(channel.to_string()))?;

        let Some(scope) = parts.next() else {
            return Err(PubSubError::InvalidChannelKey(channel.to_string()));
        };

        let topic = match scope {
            "global" => {
                if parts.next().is_some() {
                    return Err(PubSubError::InvalidChannelKey(channel.to_string()));
                }
                EventTopic::Global
            }
            "channel" => {
                let Some(channel_id) = parts.next() else {
                    return Err(PubSubError::InvalidChannelKey(channel.to_string()));
                };
                if parts.next().is_some() {
                    return Err(PubSubError::InvalidChannelKey(channel.to_string()));
                }
                EventTopic::Channel(
                    Uuid::parse_str(channel_id)
                        .map_err(|_| PubSubError::InvalidChannelKey(channel.to_string()))?,
                )
            }
            _ => return Err(PubSubError::InvalidChannelKey(channel.to_string())),
        };

        Ok(Self {
            community_id,
            topic,
        })
    }
}

/// Redis channel for exact-channel events under `ctx`.
pub fn channel_key(ctx: &TenantContext, channel_id: Uuid) -> String {
    EventTopicKey::from_context(ctx, EventTopic::Channel(channel_id)).redis_channel()
}

/// Redis channel for community-global events under `ctx`.
pub fn global_key(ctx: &TenantContext) -> String {
    EventTopicKey::from_context(ctx, EventTopic::Global).redis_channel()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[test]
    fn channel_key_includes_community_and_channel() {
        let ctx = ctx(0xaaaa, "a.example");
        let channel_id = Uuid::from_u128(0xbbbb);

        assert_eq!(
            channel_key(&ctx, channel_id),
            format!("buzz:{}:channel:{channel_id}", ctx.community())
        );
    }

    #[test]
    fn global_key_includes_community() {
        let ctx = ctx(0xaaaa, "a.example");

        assert_eq!(global_key(&ctx), format!("buzz:{}:global", ctx.community()));
    }

    #[test]
    fn same_channel_in_two_communities_has_different_topics() {
        let community_a = ctx(0xaaaa, "a.example");
        let community_b = ctx(0xbbbb, "b.example");
        let channel_id = Uuid::from_u128(0xcccc);

        assert_ne!(
            channel_key(&community_a, channel_id),
            channel_key(&community_b, channel_id)
        );
    }

    #[test]
    fn parses_channel_topic() {
        let community_id = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let channel_id = Uuid::from_u128(0xbbbb);
        let raw = format!("buzz:{community_id}:channel:{channel_id}");

        assert_eq!(
            EventTopicKey::parse_redis_channel(&raw).unwrap(),
            EventTopicKey {
                community_id,
                topic: EventTopic::Channel(channel_id),
            }
        );
    }

    #[test]
    fn parses_global_topic() {
        let community_id = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let raw = format!("buzz:{community_id}:global");

        assert_eq!(
            EventTopicKey::parse_redis_channel(&raw).unwrap(),
            EventTopicKey {
                community_id,
                topic: EventTopic::Global,
            }
        );
    }

    #[test]
    fn rejects_malformed_or_wrong_prefix_topics() {
        for raw in [
            "",
            "not-buzz:00000000-0000-0000-0000-00000000aaaa:global",
            "buzz:not-a-uuid:global",
            "buzz:00000000-0000-0000-0000-00000000aaaa",
            "buzz:00000000-0000-0000-0000-00000000aaaa:global:extra",
            "buzz:00000000-0000-0000-0000-00000000aaaa:channel",
            "buzz:00000000-0000-0000-0000-00000000aaaa:channel:not-a-uuid",
            "buzz:00000000-0000-0000-0000-00000000aaaa:presence:abc",
        ] {
            assert!(
                EventTopicKey::parse_redis_channel(raw).is_err(),
                "expected {raw:?} to be rejected"
            );
        }
    }
}
