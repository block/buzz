//! Per-agent self-update policy (#6287).
//!
//! A managed agent can send `buzz agents draft-update` for its own definition.
//! By default that draft lands as an owner-reviewed form and nothing changes
//! until the owner clicks Save. An owner may opt one agent into applying a
//! bounded set of fields on its own: the record's `self_update_fields`
//! allowlist. This module holds the pure authorization step the Tauri command
//! runs under the store lock, so the decision and the write share one
//! boundary and the test suite can pin every rejection without an app handle.
//!
//! The policy is local bookkeeping, like `auto_restart_on_config_change`: it is
//! never published on kind:30177 and never exported in agent or team snapshots.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use super::{AgentDefinition, ManagedAgentRecord, UpdatePersonaRequest};

/// A definition field an agent may change on its own draft-update.
///
/// Deliberately closed: runtime, provider, respond-to, env vars and anything
/// that changes what process runs or who the agent answers stay owner-reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfUpdateField {
    SystemPrompt,
    Model,
    DisplayName,
}

impl SelfUpdateField {
    /// Wire name, as stored in `managed-agents.json` and sent to the UI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemPrompt => "system_prompt",
            Self::Model => "model",
            Self::DisplayName => "display_name",
        }
    }
}

/// Sort and dedupe a policy so the stored bytes are canonical.
pub fn normalize_self_update_fields(fields: Vec<SelfUpdateField>) -> Vec<SelfUpdateField> {
    fields
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Lenient store reader: a name this build does not know (written by a newer
/// build) is dropped rather than failing the whole agent store. Dropping is the
/// safe direction, since an unknown name grants nothing here.
pub fn deserialize_self_update_fields<'de, D>(
    deserializer: D,
) -> Result<Vec<SelfUpdateField>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: Vec<serde_json::Value> = Vec::deserialize(deserializer)?;
    Ok(normalize_self_update_fields(
        raw.into_iter()
            .filter_map(|value| serde_json::from_value::<SelfUpdateField>(value).ok())
            .collect(),
    ))
}

/// The update draft exactly as the desktop parses it off the observer frame
/// (`parseAgentManagementRequest`): every field is optional, absent means
/// "not requested". Any field outside the allowlist that is present sends the
/// whole draft to owner review.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfUpdateDraft {
    pub channel_id: String,
    pub agent_name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub respond_to: Option<String>,
}

/// Why a draft stays on the owner-review path. Every variant is a fallback to
/// today's behaviour, never a dropped request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfUpdateRejection {
    /// The signer is not one of this owner's managed agents.
    UnknownSigner,
    /// The signer's record has no self-update policy (the default).
    PolicyEmpty,
    /// The draft sets no field at all.
    NoChanges,
    /// The draft sets a field outside the signer's allowlist.
    FieldNotAllowed(&'static str),
    /// The signer is a definition-less instance; there is no definition to edit.
    NoLinkedDefinition,
    /// The draft names a definition that is not the signer's own.
    TargetMismatch,
    /// More than one editable definition carries the requested name.
    AmbiguousTarget,
    /// The signer's definition is team-sourced and cannot be edited here.
    DefinitionNotEditable,
    /// Another instance shares the definition and does not allow the field.
    SiblingNotAllowed { pubkey: String, field: &'static str },
    /// The draft is older than [`MAX_SELF_UPDATE_DRAFT_AGE`], or undated.
    Stale,
    /// The definition changed between planning and applying.
    DefinitionChanged,
}

impl std::fmt::Display for SelfUpdateRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSigner => write!(f, "signer is not a managed agent"),
            Self::PolicyEmpty => write!(f, "agent has no self-update policy"),
            Self::NoChanges => write!(f, "draft sets no field"),
            Self::FieldNotAllowed(field) => write!(f, "field {field} is not self-updatable"),
            Self::NoLinkedDefinition => write!(f, "agent has no linked definition"),
            Self::TargetMismatch => write!(f, "draft targets a different agent"),
            Self::AmbiguousTarget => write!(f, "more than one definition has that name"),
            Self::DefinitionNotEditable => write!(f, "definition is team-sourced"),
            Self::SiblingNotAllowed { pubkey, field } => {
                write!(f, "sibling instance {pubkey} does not allow {field}")
            }
            Self::Stale => write!(f, "draft is too old"),
            Self::DefinitionChanged => write!(f, "definition changed while applying"),
        }
    }
}

/// What an accepted draft will change. Only allowlisted fields can be set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdatePlan {
    pub persona_id: String,
    /// `updated_at` of the definition the plan was built from. The apply step
    /// rejects if the stored value moved, so a concurrent owner edit wins.
    pub expected_updated_at: String,
    pub display_name: Option<String>,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
}

impl SelfUpdatePlan {
    /// Fields this plan sets, in canonical order.
    pub fn fields(&self) -> Vec<SelfUpdateField> {
        let mut fields = Vec::new();
        if self.system_prompt.is_some() {
            fields.push(SelfUpdateField::SystemPrompt);
        }
        if self.model.is_some() {
            fields.push(SelfUpdateField::Model);
        }
        if self.display_name.is_some() {
            fields.push(SelfUpdateField::DisplayName);
        }
        fields
    }
}

/// A draft older than this goes to owner review. Bounds the relay's reconnect
/// replay (five minutes) so a resurrected draft cannot re-apply silently.
pub const MAX_SELF_UPDATE_DRAFT_AGE: Duration = Duration::minutes(10);
/// Tolerated clock skew for a draft stamped slightly in the future.
pub const MAX_SELF_UPDATE_DRAFT_SKEW: Duration = Duration::minutes(2);

/// Whether a draft's envelope timestamp is recent enough to auto-apply. An
/// absent or unparseable stamp is treated as stale: review is the safe side.
pub fn draft_is_fresh(issued_at: Option<&str>, now: DateTime<Utc>) -> bool {
    let Some(issued_at) = issued_at.and_then(|raw| DateTime::parse_from_rfc3339(raw).ok()) else {
        return false;
    };
    let age = now.signed_duration_since(issued_at.with_timezone(&Utc));
    age <= MAX_SELF_UPDATE_DRAFT_AGE && age >= -MAX_SELF_UPDATE_DRAFT_SKEW
}

fn normalized_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Decide whether `signer_pubkey`'s draft may be applied without review.
///
/// Rules, in order: the signer must be a managed agent with a non-empty
/// policy; every field the draft sets must be in that policy; the draft must
/// name exactly one editable definition and it must be the signer's own; every
/// other instance sharing that definition must allow the same fields, because
/// a definition edit reaches all of them.
pub fn evaluate_self_update(
    signer_pubkey: &str,
    records: &[ManagedAgentRecord],
    definitions: &[AgentDefinition],
    draft: &SelfUpdateDraft,
) -> Result<SelfUpdatePlan, SelfUpdateRejection> {
    let signer = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(signer_pubkey))
        .ok_or(SelfUpdateRejection::UnknownSigner)?;
    if signer.self_update_fields.is_empty() {
        return Err(SelfUpdateRejection::PolicyEmpty);
    }

    let present = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
    if present(&draft.runtime) {
        return Err(SelfUpdateRejection::FieldNotAllowed("runtime"));
    }
    if present(&draft.provider) {
        return Err(SelfUpdateRejection::FieldNotAllowed("provider"));
    }
    if present(&draft.respond_to) {
        return Err(SelfUpdateRejection::FieldNotAllowed("respond_to"));
    }
    let requested: Vec<SelfUpdateField> = [
        (present(&draft.system_prompt), SelfUpdateField::SystemPrompt),
        (present(&draft.model), SelfUpdateField::Model),
        (present(&draft.display_name), SelfUpdateField::DisplayName),
    ]
    .into_iter()
    .filter_map(|(set, field)| set.then_some(field))
    .collect();
    if requested.is_empty() {
        return Err(SelfUpdateRejection::NoChanges);
    }
    if let Some(field) = requested
        .iter()
        .find(|field| !signer.self_update_fields.contains(field))
    {
        return Err(SelfUpdateRejection::FieldNotAllowed(field.as_str()));
    }

    let persona_id = signer
        .persona_id
        .as_deref()
        .ok_or(SelfUpdateRejection::NoLinkedDefinition)?;
    let target = normalized_name(&draft.agent_name);
    let mut matches = definitions
        .iter()
        .filter(|definition| normalized_name(&definition.display_name) == target);
    let definition = match (matches.next(), matches.next()) {
        (None, _) => return Err(SelfUpdateRejection::TargetMismatch),
        (Some(_), Some(_)) => return Err(SelfUpdateRejection::AmbiguousTarget),
        (Some(definition), None) => definition,
    };
    if definition.source_team.is_some() {
        return Err(SelfUpdateRejection::DefinitionNotEditable);
    }
    if definition.id != persona_id {
        return Err(SelfUpdateRejection::TargetMismatch);
    }

    for sibling in records.iter().filter(|record| {
        record.persona_id.as_deref() == Some(persona_id)
            && !record.pubkey.eq_ignore_ascii_case(&signer.pubkey)
    }) {
        if let Some(field) = requested
            .iter()
            .find(|field| !sibling.self_update_fields.contains(field))
        {
            return Err(SelfUpdateRejection::SiblingNotAllowed {
                pubkey: sibling.pubkey.clone(),
                field: field.as_str(),
            });
        }
    }

    let trimmed = |value: &Option<String>| value.as_deref().map(|v| v.trim().to_string());
    Ok(SelfUpdatePlan {
        persona_id: definition.id.clone(),
        expected_updated_at: definition.updated_at.clone(),
        display_name: trimmed(&draft.display_name),
        system_prompt: trimmed(&draft.system_prompt),
        model: trimmed(&draft.model),
    })
}

/// Project a plan onto the full `update_persona` request shape, carrying every
/// untouched field through unchanged. `env_vars` and `behavior` stay absent so
/// the stored values are not rewritten.
pub fn self_update_request(
    persona: &AgentDefinition,
    plan: &SelfUpdatePlan,
) -> UpdatePersonaRequest {
    UpdatePersonaRequest {
        id: persona.id.clone(),
        display_name: plan
            .display_name
            .clone()
            .unwrap_or_else(|| persona.display_name.clone()),
        avatar_url: persona.avatar_url.clone(),
        description: persona.description.clone(),
        system_prompt: plan
            .system_prompt
            .clone()
            .unwrap_or_else(|| persona.system_prompt.clone()),
        acp_command: persona.acp_command.clone(),
        runtime: persona.runtime.clone(),
        model: plan.model.clone().or_else(|| persona.model.clone()),
        provider: persona.provider.clone(),
        name_pool: persona.name_pool.clone(),
        env_vars: None,
        behavior: None,
    }
}

#[cfg(test)]
mod tests;
