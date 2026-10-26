//! User target validation and synthetic inventory generation.
use crate::entity_deletion::EntityRequest;
use crate::store::entity_deletion::requests::validate_operator;
use crate::{DbError, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

fn invalid(message: &str) -> DbError {
    DbError::DeletionSafety(message.into())
}
pub(super) fn validate_submission(
    target: &str,
    requested_by: &str,
    reason: Option<&str>,
) -> Result<()> {
    let slug = target.strip_prefix("demo:").unwrap_or_default();
    if slug.is_empty()
        || slug.len() > 64
        || !slug.as_bytes()[0].is_ascii_alphanumeric()
        || !slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(DbError::DeletionSafety(
            "target must be demo:<1..64 lowercase letters, digits, underscores or hyphens>".into(),
        ));
    }
    validate_operator(requested_by)?;
    if reason.is_some_and(|r| r.len() > 4096) {
        return Err(DbError::DeletionSafety("reason exceeds 4096 bytes".into()));
    }
    Ok(())
}

pub(super) fn demo_inventory(target: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "target": target,
        "communities": [
            {
                "community_id": "00000000-0000-0000-0000-00000000000b",
                "label": "demo:community-b",
                "relationship": "participant",
                "action": "delete_subject_data"
            },
            {
                "community_id": "00000000-0000-0000-0000-00000000000c",
                "label": "demo:community-c",
                "relationship": "former_participant",
                "action": "delete_subject_data"
            }
        ]
    })
}

/// Validated user-deletion inventory, shared by preparation and workflow policy.
pub struct UserDeletionInventory {
    /// Communities in approved processing order, with their saved attributes.
    pub communities: Vec<(Uuid, Value)>,
}

impl UserDeletionInventory {
    /// Validate and read a saved user inventory.
    pub fn read(request: &EntityRequest) -> Result<Self> {
        if request.entity_type != "user" {
            return Err(invalid("user workflow requires entity_type user"));
        }
        validate_submission(
            &request.target,
            &request.requested_by,
            request.reason.as_deref(),
        )?;
        let value = request
            .inventory_manifest
            .as_ref()
            .ok_or_else(|| invalid("missing inventory"))?;
        if value["version"] != 1 || value["target"] != request.target {
            return Err(invalid("unsupported inventory version or target"));
        }
        let rows = value["communities"]
            .as_array()
            .ok_or_else(|| invalid("missing communities"))?;
        if rows.len() > 1000 {
            return Err(invalid("inventory exceeds 1000 communities"));
        }
        let mut seen = HashSet::new();
        let mut communities = Vec::new();
        for row in rows {
            let id = row["community_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| invalid("invalid inventory community id"))?;
            if !seen.insert(id)
                || row["action"] != "delete_subject_data"
                || !matches!(
                    row["relationship"].as_str(),
                    Some("participant" | "former_participant")
                )
            {
                return Err(invalid("duplicate community or unsupported deletion scope"));
            }
            communities.push((id, row.clone()));
        }
        Ok(Self { communities })
    }

    /// Build handler inputs for a request-wide or community-scoped stage.
    pub fn inputs(&self, target: &str, community: Option<Uuid>) -> Result<Value> {
        let community = match community {
            None => Value::Null,
            Some(id) => self
                .communities
                .iter()
                .find(|(key, _)| *key == id)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| invalid("stage community is outside inventory"))?,
        };
        Ok(json!({"target": target, "community": community}))
    }
}
