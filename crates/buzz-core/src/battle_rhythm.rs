//! Strict wire contracts for owner-authored Battle Rhythm records.
#![allow(missing_docs)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleRhythmSourceV1 {
    pub schema_version: u8,
    pub id: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub display_name: String,
    pub coverage_start: String,
    pub coverage_end: String,
    pub document_name: String,
    pub document_hash: String,
    pub revision_id: String,
    pub prior_revision_id: Option<String>,
    pub imported_at: String,
    pub status: String,
    pub source_reference: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum EventOwnershipV1 {
    Manual,
    Source {
        source_id: String,
        revision_id: String,
        source_location: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleRhythmEventV1 {
    pub schema_version: u8,
    pub id: String,
    pub ownership: EventOwnershipV1,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub event_type: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub time_zone: String,
    pub status: String,
    pub location: Option<String>,
    pub responsible_owner: Option<String>,
    pub participants: Vec<String>,
    pub remarks: Option<String>,
    pub linked_plan_id: Option<String>,
    pub linked_task_id: Option<String>,
    pub linked_mission_requirement_id: Option<String>,
    pub parent_activity_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BattleRhythmRevisionChunkV1 {
    pub schema_version: u8,
    pub revision_id: String,
    pub source_id: String,
    pub prior_revision_id: Option<String>,
    pub imported_at: String,
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub manifest_hash: String,
    pub changes: Vec<serde_json::Value>,
}
