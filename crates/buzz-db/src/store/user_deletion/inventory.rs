//! Validation of saved inventories, independent of the intake identity format.
use super::MAX_COMMUNITIES;
use crate::entity_deletion::EntityRequest;
use crate::{DbError, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

/// Bound per-community agent progress stored in one stage row.
pub const MAX_AGENTS_PER_COMMUNITY: usize = 1024;
/// Bound the total approved agent scope in a request.
pub const MAX_INVENTORY_AGENTS: usize = 4096;

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
        // Preserve old manifests and their exact saved handler inputs on resume.
        let legacy = value.get("version").is_some() || value.get("target").is_some();
        if legacy && (value["version"] != 1 || value["target"] != request.target) {
            return Err(invalid("unsupported inventory version or target"));
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
        let mut agent_count = 0;
        for row in rows {
            if row["relationship"] == "owner" {
                return Err(invalid("owned-community execution is not supported yet"));
            }
            let id = row["community_id"]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| invalid("invalid inventory community id"))?;
            if !seen.insert(id)
                || (legacy && row["action"] != "delete_subject_data")
                || (!legacy && row.get("action").is_some())
                || !matches!(
                    row["relationship"].as_str(),
                    Some("participant" | "former_participant")
                )
            {
                return Err(invalid("duplicate community or unsupported deletion scope"));
            }
            if row.get("agents").is_some() {
                agent_count += read_agents(row, &request.target)?.len();
                if agent_count > MAX_INVENTORY_AGENTS {
                    return Err(invalid("inventory exceeds 4096 agents"));
                }
            }
            communities.push((id, row.clone()));
        }
        Ok(Self { communities })
    }

    /// Read the exact approved agents for one community. Legacy inventories with
    /// no agent field remain readable, but cannot authorize agent cleanup.
    pub fn agents(&self, community: Uuid, owner: &str) -> Result<Vec<String>> {
        let (_, row) = self
            .communities
            .iter()
            .find(|(id, _)| *id == community)
            .ok_or_else(|| invalid("agent community is outside inventory"))?;
        read_agents(row, owner)
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

fn read_agents(row: &Value, owner: &str) -> Result<Vec<String>> {
    let entries = row["agents"].as_array().ok_or_else(|| {
        invalid("approved inventory has no agent list; prepare and approve agent scope first")
    })?;
    if entries.len() > MAX_AGENTS_PER_COMMUNITY {
        return Err(invalid("inventory exceeds 1024 agents in one community"));
    }
    let mut seen = HashSet::new();
    entries
        .iter()
        .map(|entry| {
            let key = entry["pubkey"]
                .as_str()
                .ok_or_else(|| invalid("invalid inventory agent"))?;
            let normalized = super::normalize_user_deletion_pubkey(key)?;
            if key != normalized || key == owner || !seen.insert(key) {
                return Err(invalid(
                    "agent keys must be canonical, unique and different from the owner",
                ));
            }
            Ok(key.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_scope_requires_an_explicit_bounded_canonical_list() {
        let owner = "01".repeat(32);
        let agent = "ab".repeat(32);
        assert!(read_agents(&json!({}), &owner).is_err());
        assert!(read_agents(&json!({"agents": []}), &owner)
            .unwrap()
            .is_empty());
        assert_eq!(
            read_agents(&json!({"agents": [{"pubkey": agent}]}), &owner).unwrap(),
            vec![agent.clone()]
        );
        for entries in [
            json!([{"pubkey": owner}]),
            json!([{"pubkey": agent}, {"pubkey": agent}]),
            json!([{"pubkey": agent.to_uppercase()}]),
            json!([{"pubkey": "not a public key"}]),
            json!([{}]),
            json!(null),
            json!(vec![json!({"pubkey": agent}); MAX_AGENTS_PER_COMMUNITY + 1]),
        ] {
            assert!(read_agents(&json!({"agents": entries}), &owner).is_err());
        }
    }
}
