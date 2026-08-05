//! Provider-neutral authorization invalidation hints over Redis pub/sub.
//!
//! Hints contain no selector or identity data. The community is derived from
//! the server-owned Redis channel and the durable generation is reconciled
//! from Postgres by every consumer.

use buzz_core::CommunityId;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::topic::BUZZ_PREFIX;

/// Current provider-neutral hint wire version.
pub const AUTHORIZATION_INVALIDATION_WIRE_VERSION: u16 = 1;
/// Tenant-local Redis channel suffix.
pub const AUTHORIZATION_INVALIDATION_SUFFIX: &str = "authorization-invalidation";
/// Pattern subscribed by relay nodes.
pub const AUTHORIZATION_INVALIDATION_PATTERN: &str = "buzz:*:authorization-invalidation";

/// Redis hint that a durable domain generation may have advanced.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizationInvalidationHint {
    /// Envelope version. Unknown versions trigger a full durable reconcile.
    pub wire_version: u16,
    /// Highest generation known by the publisher after its commit.
    pub generation: u64,
}

impl AuthorizationInvalidationHint {
    /// Construct a current-version hint for a positive durable generation.
    pub const fn current(generation: u64) -> Self {
        Self {
            wire_version: AUTHORIZATION_INVALIDATION_WIRE_VERSION,
            generation,
        }
    }
}

/// A hint scoped by its server-owned Redis channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopedAuthorizationInvalidationHint {
    /// Authorization domain parsed from the channel.
    pub community_id: CommunityId,
    /// Provider-neutral durable-generation hint.
    pub hint: AuthorizationInvalidationHint,
}

/// Redis channel for one server-resolved authorization domain.
pub fn authorization_invalidation_channel(community_id: CommunityId) -> String {
    format!("{BUZZ_PREFIX}:{community_id}:{AUTHORIZATION_INVALIDATION_SUFFIX}")
}

/// Parse the exact authorization-invalidation channel shape.
pub fn parse_authorization_invalidation_channel(channel: &str) -> Option<CommunityId> {
    let mut parts = channel.split(':');
    if parts.next()? != BUZZ_PREFIX {
        return None;
    }
    let community_id = Uuid::parse_str(parts.next()?).ok()?;
    if parts.next()? != AUTHORIZATION_INVALIDATION_SUFFIX || parts.next().is_some() {
        return None;
    }
    Some(CommunityId::from_uuid(community_id))
}

const BACKOFF_INITIAL_SECS: u64 = 1;
const BACKOFF_MAX_SECS: u64 = 30;

/// Subscribe forever, reconnecting with bounded exponential backoff.
pub async fn run_authorization_invalidation_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<ScopedAuthorizationInvalidationHint>,
) {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;
    loop {
        match connect_and_subscribe(&redis_url, &broadcast_tx).await {
            Ok(()) => {
                backoff_secs = BACKOFF_INITIAL_SECS;
                tracing::warn!(
                    "Redis authorization-invalidation stream ended; reconnecting in {backoff_secs}s"
                );
            }
            Err(error) => tracing::error!(
                "Redis authorization-invalidation error: {error}; reconnecting in {backoff_secs}s"
            ),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(BACKOFF_MAX_SECS);
    }
}

async fn connect_and_subscribe(
    redis_url: &str,
    broadcast_tx: &broadcast::Sender<ScopedAuthorizationInvalidationHint>,
) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let mut connection = client.get_async_pubsub().await?;
    connection
        .psubscribe(AUTHORIZATION_INVALIDATION_PATTERN)
        .await?;
    tracing::info!(
        "Redis authorization-invalidation subscriber listening on {AUTHORIZATION_INVALIDATION_PATTERN}"
    );

    let mut stream = connection.on_message();
    while let Some(message) = stream.next().await {
        let channel = message.get_channel_name();
        let Some(community_id) = parse_authorization_invalidation_channel(channel) else {
            tracing::warn!("ignoring authorization-invalidation hint on unexpected channel");
            continue;
        };
        let payload: String = match message.get_payload() {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "ignoring unreadable authorization-invalidation hint");
                continue;
            }
        };
        let hint: AuthorizationInvalidationHint = match serde_json::from_str(&payload) {
            Ok(hint) => hint,
            Err(error) => {
                tracing::warn!(%error, "ignoring malformed authorization-invalidation hint");
                continue;
            }
        };
        if broadcast_tx
            .send(ScopedAuthorizationInvalidationHint { community_id, hint })
            .is_err()
        {
            tracing::trace!("no local authorization-invalidation receivers");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn channels_are_exactly_domain_scoped() {
        let a = CommunityId::from_uuid(Uuid::from_u128(0xaaaa));
        let b = CommunityId::from_uuid(Uuid::from_u128(0xbbbb));
        assert_ne!(
            authorization_invalidation_channel(a),
            authorization_invalidation_channel(b)
        );
        assert_eq!(
            parse_authorization_invalidation_channel(
                authorization_invalidation_channel(a).as_str()
            ),
            Some(a)
        );
    }

    #[test]
    fn rejects_ambiguous_channels() {
        for channel in [
            "buzz:authorization-invalidation",
            "buzz:not-a-uuid:authorization-invalidation",
            "buzz:00000000-0000-0000-0000-00000000aaaa:authorization-invalidation:extra",
            "other:00000000-0000-0000-0000-00000000aaaa:authorization-invalidation",
        ] {
            assert_eq!(parse_authorization_invalidation_channel(channel), None);
        }
    }

    #[test]
    fn envelope_roundtrip_contains_only_version_and_generation() {
        let payload = serde_json::to_string(&AuthorizationInvalidationHint::current(42))
            .expect("hint serializes");
        assert_eq!(payload, r#"{"wire_version":1,"generation":42}"#);
        assert_eq!(
            serde_json::from_str::<AuthorizationInvalidationHint>(&payload)
                .expect("hint deserializes"),
            AuthorizationInvalidationHint::current(42)
        );
    }

    #[test]
    fn unknown_wire_version_remains_visible_to_reconciler() {
        let hint: AuthorizationInvalidationHint =
            serde_json::from_str(r#"{"wire_version":99,"generation":7}"#)
                .expect("forward version remains decodable");
        assert_eq!(hint.wire_version, 99);
        assert_eq!(hint.generation, 7);
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn redis_roundtrip_preserves_only_scoped_generation_hint() {
        let pool = crate::test_util::make_test_pool();
        let manager = Arc::new(
            crate::PubSubManager::new("redis://127.0.0.1:6379", pool)
                .await
                .expect("create pubsub manager"),
        );
        let mut receiver = manager.subscribe_authorization_invalidations();
        let subscriber = manager.clone();
        let task =
            tokio::spawn(
                async move { subscriber.run_authorization_invalidation_subscriber().await },
            );
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        manager
            .publish_authorization_invalidation(
                community_id,
                AuthorizationInvalidationHint::current(11),
            )
            .await
            .expect("publish hint");
        let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), receiver.recv())
            .await
            .expect("hint arrives")
            .expect("broadcast remains open");
        assert_eq!(received.community_id, community_id);
        assert_eq!(received.hint, AuthorizationInvalidationHint::current(11));
        task.abort();
    }
}
