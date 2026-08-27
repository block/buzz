//! Stage-one owner import and reader-specific section-workspace projection.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;
pub const VERSION: u32 = 1;
pub const MAX_SECTIONS: usize = 100;
pub const MAX_ASSIGNMENTS: usize = 1_000;
pub const MAX_ENCRYPTED_METADATA_BYTES: usize = 65_535;
pub const MAX_KEY_ENVELOPE_BYTES: usize = 4_096;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSection {
    pub id: Uuid,
    pub rank: u32,
    pub encrypted_label: String,
    pub encrypted_icon: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportAssignment {
    pub channel_id: Uuid,
    pub section_id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub version: u32,
    pub action_id: Uuid,
    pub source_event_id: String,
    pub source_hash: String,
    pub key_epoch: u64,
    pub owner_key_envelope: String,
    pub sections: Vec<ImportSection>,
    pub assignments: Vec<ImportAssignment>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationMarker {
    pub source_event_id: String,
    pub source_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedAssignment {
    pub channel_id: Uuid,
    pub section_id: Uuid,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub version: u32,
    pub owner_pubkey: String,
    pub revision: u64,
    pub layout_revision: u64,
    pub key_epoch: u64,
    pub migration: MigrationMarker,
    pub reader_key_envelope: String,
    pub sections: Vec<ImportSection>,
    pub assignments: Vec<ProjectedAssignment>,
}
impl Import {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != VERSION {
            return Err(ValidationError::Invalid("version"));
        }
        if self.action_id.is_nil() {
            return Err(ValidationError::Invalid("action_id"));
        }
        if self.key_epoch != 1 {
            return Err(ValidationError::Invalid("key_epoch"));
        }
        hex32(&self.source_event_id, "source_event_id")?;
        hex32(&self.source_hash, "source_hash")?;
        bounded(
            &self.owner_key_envelope,
            MAX_KEY_ENVELOPE_BYTES,
            "owner_key_envelope",
        )?;
        if self.sections.len() > MAX_SECTIONS {
            return Err(ValidationError::TooMany("sections", MAX_SECTIONS));
        }
        if self.assignments.len() > MAX_ASSIGNMENTS {
            return Err(ValidationError::TooMany("assignments", MAX_ASSIGNMENTS));
        }
        let mut ids = HashSet::with_capacity(self.sections.len());
        let mut ranks = HashSet::with_capacity(self.sections.len());
        for section in &self.sections {
            if section.id.is_nil() || !ids.insert(section.id) {
                return Err(ValidationError::Invalid("section id"));
            }
            if section.rank as usize >= self.sections.len() || !ranks.insert(section.rank) {
                return Err(ValidationError::Invalid("section rank"));
            }
            bounded(
                &section.encrypted_label,
                MAX_ENCRYPTED_METADATA_BYTES,
                "encrypted_label",
            )?;
            if let Some(icon) = &section.encrypted_icon {
                bounded(icon, MAX_ENCRYPTED_METADATA_BYTES, "encrypted_icon")?;
            }
        }
        let mut channels = HashSet::with_capacity(self.assignments.len());
        if self.assignments.iter().any(|assignment| {
            assignment.channel_id.is_nil()
                || !channels.insert(assignment.channel_id)
                || !ids.contains(&assignment.section_id)
        }) {
            return Err(ValidationError::Invalid("assignment"));
        }
        Ok(())
    }
    pub fn owner_projection(&self, owner_pubkey: String) -> Projection {
        Projection {
            version: VERSION,
            owner_pubkey,
            revision: 1,
            layout_revision: 1,
            key_epoch: self.key_epoch,
            migration: MigrationMarker {
                source_event_id: self.source_event_id.clone(),
                source_hash: self.source_hash.clone(),
            },
            reader_key_envelope: self.owner_key_envelope.clone(),
            sections: self.sections.clone(),
            assignments: self
                .assignments
                .iter()
                .map(|assignment| ProjectedAssignment {
                    channel_id: assignment.channel_id,
                    section_id: assignment.section_id,
                    revision: 1,
                })
                .collect(),
        }
    }
}
pub fn validate_import_tags(
    tags: &[nostr::Tag],
    owner: &str,
    action_id: Uuid,
) -> Result<(), ValidationError> {
    let expected_action = action_id.to_string();
    let fields: Vec<Vec<String>> = tags.iter().map(|tag| tag.as_slice().to_vec()).collect();
    if fields
        != [
            vec!["p".into(), owner.into()],
            vec!["action".into(), expected_action],
        ]
    {
        return Err(ValidationError::Invalid("tags"));
    }
    Ok(())
}
fn hex32(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ValidationError::Invalid(field));
    }
    Ok(())
}
fn bounded(value: &str, max: usize, field: &'static str) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max {
        return Err(ValidationError::Invalid(field));
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("{0} exceeds maximum {1}")]
    TooMany(&'static str, usize),
}
