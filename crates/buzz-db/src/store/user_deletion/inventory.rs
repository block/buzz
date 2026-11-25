//! Validation of saved inventories, independent of the intake identity format.
use super::MAX_COMMUNITIES;
use crate::entity_deletion::EntityRequest;
use crate::{DbError, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

fn invalid(message: &str) -> DbError {
    DbError::DeletionSafety(message.into())
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
        let value = request
            .inventory_manifest
            .as_ref()
            .ok_or_else(|| invalid("missing inventory"))?;
        if value["workflow_version"] != 2 {
            return Err(invalid("unsupported user deletion workflow version; inspect the saved request before proceeding"));
        }
        let rows = value["communities"]
            .as_array()
            .ok_or_else(|| invalid("missing communities"))?;
        if rows.len() > MAX_COMMUNITIES {
            return Err(invalid(&format!(
                "inventory exceeds {MAX_COMMUNITIES} communities"
            )));
        }
        let mut seen = HashSet::new();
        let mut communities = Vec::new();
        for row in rows {
            let id = row["community_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| invalid("invalid inventory community id"))?;
            if !seen.insert(id)
                || row.get("action").is_some()
                || !matches!(
                    row["relationship"].as_str(),
                    Some("participant" | "former_participant" | "past_participant" | "owner")
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
    pub(super) fn non_owner_inputs(&self, target: &str, community: Option<Uuid>) -> Result<Value> {
        let community = community.ok_or_else(|| invalid("missing stage community"))?;
        let inputs = self.inputs(target, Some(community))?;
        if inputs["community"]["relationship"] == "owner" {
            return Err(invalid(
                "owner inventory requires the community deletion handoff",
            ));
        }
        Ok(inputs)
    }
}
