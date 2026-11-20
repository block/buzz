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
    /// Check the persisted execution mode, including legacy rehearsal identities.
    pub fn is_rehearsal(request: &EntityRequest) -> Result<bool> {
        let manifest = request.inventory_manifest.as_ref();
        let mode = manifest.and_then(|v| v.get("execution_mode"));
        match (request.target.strip_prefix("rehearsal:"), mode) {
            (Some(identity), Some(mode)) if mode == "rehearsal" => {
                let subject = if let Some((run, subject)) = identity.split_once(':') {
                    if manifest.is_none_or(|v| v["workflow_version"] != 2) {
                        return Err(invalid("unsupported rehearsal workflow version"));
                    }
                    let run =
                        Uuid::parse_str(run).map_err(|_| invalid("invalid rehearsal run ID"))?;
                    if manifest.and_then(|v| v["run_id"].as_str()) != Some(run.to_string().as_str())
                    {
                        return Err(invalid("rehearsal run ID differs from inventory"));
                    }
                    subject
                } else {
                    identity
                };
                if super::normalize_user_deletion_pubkey(subject)? != subject {
                    return Err(invalid("noncanonical rehearsal subject"));
                }
                Ok(true)
            }
            (None, None) => Ok(false),
            _ => Err(invalid("request identity and execution mode disagree")),
        }
    }

    /// Source public key, distinct from the rehearsal's opaque request target.
    pub fn subject(request: &EntityRequest) -> Result<&str> {
        if Self::is_rehearsal(request)? {
            request
                .target
                .rsplit(':')
                .next()
                .ok_or_else(|| invalid("missing subject"))
        } else {
            Ok(&request.target)
        }
    }

    /// Validate and read a saved user inventory.
    pub fn read(request: &EntityRequest) -> Result<Self> {
        if request.entity_type != "user" {
            return Err(invalid("user workflow requires entity_type user"));
        }
        let rehearsal = Self::is_rehearsal(request)?;
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
        for row in rows {
            if row["relationship"] == "stored_community" && !rehearsal {
                return Err(invalid(
                    "all-community inspection is only supported for rehearsals",
                ));
            }
            if row["relationship"] == "owner" && !rehearsal {
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
                    Some("participant" | "former_participant" | "owner" | "stored_community")
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
