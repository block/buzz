//! Team record and team command request types, split from `types.rs`
//! (file-size cap) as the sibling of [`super::requests`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::TeamCatalogSource;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Runtime-layered instructions shared by every member deployment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub persona_ids: Vec<String>,
    #[serde(default)]
    pub is_builtin: bool,
    /// Whether this team is discoverable in the currently active community.
    /// View projection recomputed from the relay+owner-scoped kind:30178 head
    /// on every read — see [`super::AgentDefinition::shared`].
    #[serde(default)]
    pub shared: bool,
    /// Provenance of a team copied from another owner's shared catalog.
    ///
    /// Set only on the copy, never on the original. It is the sole link back
    /// to the publication — the copy carries a fresh local id — so it is what
    /// makes a repeated add idempotent instead of minting a second team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_source: Option<TeamCatalogSource>,
    /// Absolute path to the team's backing directory (if directory-backed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_dir: Option<PathBuf>,
    /// Whether `source_dir` is a symlink to an external directory.
    #[serde(default)]
    pub is_symlink: bool,
    /// Resolved symlink target path (for display). Only set when `is_symlink` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
    /// Version from the team's `plugin.json` manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRequest {
    pub name: String,
    pub description: Option<String>,
    pub instructions: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTeamRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub instructions: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
}
