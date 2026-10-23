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
    /// The definition's system prompt (`--system-prompt`).
    SystemPrompt,
    /// The definition's model id (`--model`); the provider stays as it is.
    Model,
    /// The definition's display name (`--display-name`); linked instances
    /// still carrying the old name are renamed with it.
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
/// whole draft to owner review. Unknown keys are refused outright
/// (`deny_unknown_fields`), so a field added on the TypeScript side without a
/// Rust twin forces review instead of being applied blind.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelfUpdateDraft {
    /// Channel the agent sent the draft from; audit only, not authorization.
    pub channel_id: String,
    /// Display name of the definition the draft targets.
    pub agent_name: String,
    /// New display name, when the draft sets one.
    #[serde(default)]
    pub display_name: Option<String>,
    /// New system prompt, when the draft sets one.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Never self-updatable; present means review.
    #[serde(default)]
    pub runtime: Option<String>,
    /// Never self-updatable; present means review.
    #[serde(default)]
    pub provider: Option<String>,
    /// New model id, when the draft sets one.
    #[serde(default)]
    pub model: Option<String>,
    /// Never self-updatable; present means review.
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
    /// The definition was written at or after the draft was issued, so the
    /// draft predates what it would overwrite. This is what stops a relay
    /// replay after a Desktop restart: the first apply bumps `updated_at`
    /// past the draft's timestamp, and an owner revert does the same.
    PredatesDefinition,
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
            Self::PredatesDefinition => {
                write!(f, "definition was changed after the draft was sent")
            }
            Self::DefinitionChanged => write!(f, "definition changed while applying"),
        }
    }
}

/// What an accepted draft will change. Only allowlisted fields can be set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfUpdatePlan {
    /// Id of the definition (persona) the plan edits.
    pub persona_id: String,
    /// `updated_at` of the definition the plan was built from. The apply step
    /// rejects if the stored value moved, so a concurrent owner edit wins.
    pub expected_updated_at: String,
    /// New display name, trimmed, when the draft set one.
    pub display_name: Option<String>,
    /// New system prompt, trimmed, when the draft set one.
    pub system_prompt: Option<String>,
    /// New model id, trimmed, when the draft set one.
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

/// A draft older than this goes to owner review. This is a coarse age cap
/// only; it is wider than the observer subscription's five-minute reconnect
/// replay, so it does NOT stop a replay by itself. Replay protection is the
/// `PredatesDefinition` rule in [`evaluate_self_update`]: a draft issued at or
/// before the definition's last write is refused, and every apply is a write.
pub const MAX_SELF_UPDATE_DRAFT_AGE: Duration = Duration::minutes(10);
/// Tolerated clock skew for a draft stamped slightly in the future.
pub const MAX_SELF_UPDATE_DRAFT_SKEW: Duration = Duration::minutes(2);

/// Parse a draft's envelope timestamp when it is recent enough to auto-apply.
/// An absent, unparseable, too-old or too-far-future stamp yields `None`:
/// review is the safe side.
pub fn fresh_draft_issued_at(issued_at: Option<&str>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let issued_at = DateTime::parse_from_rfc3339(issued_at?)
        .ok()?
        .with_timezone(&Utc);
    let age = now.signed_duration_since(issued_at);
    (age <= MAX_SELF_UPDATE_DRAFT_AGE && age >= -MAX_SELF_UPDATE_DRAFT_SKEW).then_some(issued_at)
}

fn normalized_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Decide whether `signer_pubkey`'s draft, issued at `issued_at`, may be
/// applied without review.
///
/// Rules, in order: the signer must be a managed agent with a non-empty
/// policy; every field the draft sets must be in that policy; the draft must
/// name exactly one editable definition and it must be the signer's own; the
/// definition must not have been written at or after `issued_at` (so a relay
/// replay of an already-applied draft, or a draft older than an owner's edit,
/// is refused); every other instance sharing that definition must allow the
/// same fields, because a definition edit reaches all of them.
pub fn evaluate_self_update(
    signer_pubkey: &str,
    records: &[ManagedAgentRecord],
    definitions: &[AgentDefinition],
    draft: &SelfUpdateDraft,
    issued_at: DateTime<Utc>,
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
    // An unparseable `updated_at` cannot prove the draft is newer, so it is
    // treated as a write that happened after the draft.
    let last_write = DateTime::parse_from_rfc3339(&definition.updated_at)
        .ok()
        .map(|stamp| stamp.with_timezone(&Utc));
    if last_write.is_none_or(|last_write| last_write >= issued_at) {
        return Err(SelfUpdateRejection::PredatesDefinition);
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
