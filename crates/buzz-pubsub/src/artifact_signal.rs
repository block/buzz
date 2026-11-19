//! Compact community artifact invalidations; never signed event payloads.
use buzz_core::{CommunityId, TenantContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Why a community artifact identity needs a fresh authorized head read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactChange {
    /// A revision committed, including an artifact deletion revision.
    Changed,
    /// The current head was redacted; an exact copy must resync.
    Redacted,
}

/// Bounded metadata for one immutable type and artifact identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSignal {
    /// Scoped artifact identity.
    pub d: Uuid,
    /// Immutable artifact type.
    #[serde(rename = "type")]
    pub artifact_type: String,
    /// Change or current-head redaction.
    pub cause: ArtifactChange,
}
impl ArtifactSignal {
    pub(crate) fn decode(payload: &str) -> Result<Self, crate::PubSubError> {
        let signal: Self = serde_json::from_str(payload)?;
        buzz_core::artifact::validate_type(&signal.artifact_type)
            .map_err(|_| crate::PubSubError::InvalidChannelKey("invalid artifact type".into()))?;
        Ok(signal)
    }
}

/// A signal bound to the server-resolved Redis topic's tenant.
#[derive(Debug, Clone)]
pub struct ScopedArtifactSignal {
    /// Tenant from the topic, never from the payload.
    pub community_id: CommunityId,
    /// Compact change metadata.
    pub signal: ArtifactSignal,
}

/// Publish compact metadata only on the dedicated tenant artifact topic.
pub async fn publish(
    pool: &crate::RedisConnection,
    tenant: &TenantContext,
    signal: &ArtifactSignal,
) -> Result<i64, crate::PubSubError> {
    let key = crate::EventTopicKey::from_context(tenant, crate::EventTopic::CommunityArtifact)
        .redis_channel();
    Ok(redis::cmd("PUBLISH")
        .arg(key)
        .arg(serde_json::to_string(signal)?)
        .query_async(&mut pool.clone())
        .await?)
}
