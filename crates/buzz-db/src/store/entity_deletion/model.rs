use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{deletion::ClaimedDeletion, Result};

use super::safety;

/// A participant community in the fixed synthetic inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoCommunity {
    /// Synthetic provenance; no real community row is required.
    pub id: Uuid,
    /// Reserved example hostname.
    pub host: String,
    /// Only participant relationships are supported by this demonstration.
    pub relationship: String,
}

/// An ordered action in the frozen, approved demonstration plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedStage {
    /// None identifies a request-wide step.
    pub target_community_id: Option<Uuid>,
    /// Explicit action selected by the dummy handler.
    pub stage: String,
}

/// Fixed three-community fixture; never a discovery result for a real user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoInventory {
    /// Fixture format version.
    pub version: u32,
    /// Synthetic target, prefixed with `demo:`.
    pub target: String,
    /// Three non-owned participant communities.
    pub communities: Vec<DemoCommunity>,
    /// Bounded deterministic execution order included in the approval digest.
    pub stages: Vec<PlannedStage>,
}

impl DemoInventory {
    /// Build the only supported fixture, without reading user or membership data.
    pub fn for_target(target: &str) -> Result<Self> {
        let slug = target.strip_prefix("demo:").unwrap_or_default();
        if slug.is_empty()
            || slug.len() > 64
            || !slug.as_bytes()[0].is_ascii_alphanumeric()
            || !slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(safety(
                "target must be demo:<1..64 lowercase letters, digits, underscores or hyphens>",
            ));
        }
        let communities: Vec<_> = (1..=3)
            .map(|n| DemoCommunity {
                id: Uuid::from_u128(n),
                host: format!("participant-{n}.invalid"),
                relationship: "participant".into(),
            })
            .collect();
        let mut stages = vec![PlannedStage {
            target_community_id: None,
            stage: "gate_user".into(),
        }];
        for community in &communities {
            stages.push(PlannedStage {
                target_community_id: Some(community.id),
                stage: "gate_community".into(),
            });
        }
        for community in &communities {
            for stage in ["drain", "delete_data", "verify_community"] {
                stages.push(PlannedStage {
                    target_community_id: Some(community.id),
                    stage: stage.into(),
                });
            }
        }
        for stage in ["verify_user", "post_delete"] {
            stages.push(PlannedStage {
                target_community_id: None,
                stage: stage.into(),
            });
        }
        Ok(Self {
            version: 1,
            target: target.into(),
            communities,
            stages,
        })
    }

    /// Reject edited fixtures, including owned communities or arbitrary actions.
    pub fn validate(&self) -> Result<()> {
        if *self != Self::for_target(&self.target)? {
            return Err(safety(
                "only the fixed non-owned participant demo inventory is supported",
            ));
        }
        Ok(())
    }

    /// Digest the validated typed manifest so JSON object ordering is irrelevant.
    pub fn digest(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(Sha256::digest(serde_json::to_vec(self)?).to_vec())
    }
}

/// Overall lifecycle and the sole ownership record for a demonstration.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EntityRequest {
    /// Request identifier.
    pub id: Uuid,
    /// Currently always user.
    pub entity_type: String,
    /// Synthetic user label.
    pub target: String,
    /// Overall lifecycle; granular progress is in stage history.
    pub status: String,
    /// Submitting operator.
    pub requested_by: String,
    /// Operator's optional reason.
    pub reason: Option<String>,
    /// Frozen synthetic inventory and stage plan.
    pub inventory_manifest: Option<Value>,
    /// Digest covered by approval.
    pub inventory_digest: Option<Vec<u8>>,
    /// Inventory freeze time.
    pub inventory_frozen_at: Option<DateTime<Utc>>,
    /// Executor holding the parent lease.
    pub lease_owner: Option<String>,
    /// Monotonic ownership generation.
    pub lease_generation: i64,
    /// Ownership deadline, checked using database time.
    pub lease_until: Option<DateTime<Utc>>,
    /// Total claims.
    pub attempts: i32,
    /// Consecutive failures of the current stage.
    pub retry_count: i32,
    /// Earliest automatic retry.
    pub next_attempt_at: DateTime<Utc>,
    /// Latest workflow error.
    pub last_error: Option<String>,
    /// Latest error time.
    pub last_error_at: Option<DateTime<Utc>>,
    /// Manual intervention marker.
    pub blocked_at: Option<DateTime<Utc>>,
    /// Why automatic attempts stopped.
    pub blocked_reason: Option<String>,
    /// Submission time.
    pub created_at: DateTime<Utc>,
    /// Last lifecycle change.
    pub updated_at: DateTime<Utc>,
    /// Demonstration completion time.
    pub completed_at: Option<DateTime<Utc>>,
}

impl EntityRequest {
    /// Decode and validate the approved fixture before dispatching any stage.
    pub fn inventory(&self) -> Result<DemoInventory> {
        let inventory: DemoInventory = serde_json::from_value(
            self.inventory_manifest
                .clone()
                .ok_or_else(|| safety("inventory is not prepared"))?,
        )?;
        if inventory.target != self.target
            || self.inventory_digest.as_deref() != Some(inventory.digest()?.as_slice())
        {
            return Err(safety("inventory does not match request target/digest"));
        }
        Ok(inventory)
    }
}

/// Ownership proof carried by user workflow operations; no community-wide fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityLeaseToken {
    /// Parent request.
    pub request_id: Uuid,
    /// Executor identity.
    pub owner: String,
    /// Claim generation.
    pub generation: i64,
}

/// The same claim container used by community deletion, with user record types.
pub type EntityClaim = ClaimedDeletion<EntityRequest, EntityLeaseToken>;

/// Current or completed granular stage, with one row reused across retries.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StageRecord {
    /// Stage record identifier.
    pub id: Uuid,
    /// Parent request.
    pub request_id: Uuid,
    /// One-based position in the approved plan.
    pub sequence: i32,
    /// Community provenance, or None for request-wide work.
    pub target_community_id: Option<Uuid>,
    /// Action name.
    pub stage: String,
    /// pending, running, failed, or completed.
    pub status: String,
    /// The request's one current unfinished stage.
    pub is_current: bool,
    /// First attempt start time.
    pub started_at: Option<DateTime<Utc>>,
    /// Successful completion time.
    pub finished_at: Option<DateTime<Utc>>,
    /// Stage details and result.
    pub metadata_json: Value,
    /// Last stage failure, retained for diagnosis after retry.
    pub last_error: Option<String>,
    /// Latest stage record update.
    pub updated_at: DateTime<Utc>,
}

/// Independent approval of the exact frozen inventory.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Approval {
    /// Parent request.
    pub request_id: Uuid,
    /// Approved manifest digest.
    pub inventory_digest: Vec<u8>,
    /// Approving operator.
    pub approved_by: String,
    /// Approval time.
    pub approved_at: DateTime<Utc>,
}

/// Consistent request, approval and stage-history snapshot for inspection.
#[derive(Debug, Serialize)]
pub struct Inspection {
    /// Overall lifecycle and ownership.
    pub request: EntityRequest,
    /// Approval, if recorded.
    pub approval: Option<Approval>,
    /// Reached stages in plan order.
    pub stage_history: Vec<StageRecord>,
}
