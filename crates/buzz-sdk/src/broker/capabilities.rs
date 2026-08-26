//! Broker capabilities — the named business operations a host will perform.
//!
//! A capability is the unit of policy: `agents.create`, `agents.update`, and
//! `agents.delete` are separate names so a host can permit one without
//! permitting the others. There is deliberately no `agents.manage` action
//! union, which would collapse three policy decisions into one.
//!
//! Every args type is `deny_unknown_fields`, so an api key, env-var map, or
//! nsec smuggled into a payload fails to deserialize instead of reaching a
//! mutation. Every outcome type can structurally hold only public identifiers —
//! it cannot carry the owner nsec or a freshly minted agent secret.

use serde::{Deserialize, Serialize};

use crate::SdkError;

/// Maximum characters in a display name or agent name.
pub const MAX_NAME_CHARS: usize = 120;

/// Maximum characters in a system prompt.
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// Maximum characters in a short scalar field (runtime, provider, model).
pub const MAX_SCALAR_CHARS: usize = 300;

/// Inbound author gate modes a requester may ask for.
///
/// `allowlist` is deliberately absent: it needs a pubkey list this narrow
/// request shape does not carry, and a mode without its list would mint an
/// agent nobody can talk to.
pub const RESPOND_TO_MODES: [&str; 2] = ["owner-only", "anyone"];

/// A capability name the broker can dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Mint a managed agent and attach it to a channel.
    AgentsCreate,
    /// Patch one managed agent belonging to the owner.
    AgentsUpdate,
    /// Remove one managed agent belonging to the owner.
    AgentsDelete,
}

impl Capability {
    /// Stable wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentsCreate => "agents.create",
            Self::AgentsUpdate => "agents.update",
            Self::AgentsDelete => "agents.delete",
        }
    }

    /// The capability contract version this build implements.
    #[must_use]
    pub fn current_version(self) -> u16 {
        match self {
            Self::AgentsCreate | Self::AgentsUpdate | Self::AgentsDelete => 1,
        }
    }

    /// Resolve a wire name.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an unknown capability name.
    pub fn parse(name: &str) -> Result<Self, SdkError> {
        match name {
            "agents.create" => Ok(Self::AgentsCreate),
            "agents.update" => Ok(Self::AgentsUpdate),
            "agents.delete" => Ok(Self::AgentsDelete),
            other => Err(SdkError::InvalidInput(format!(
                "unknown broker capability \"{other}\""
            ))),
        }
    }
}

/// Which agent an update or delete targets.
///
/// Exactly one selector, because a request naming the target twice is ambiguous
/// and the host would have to guess which wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentTarget {
    /// Target by agent pubkey (64 lowercase hex characters).
    Pubkey(String),
    /// Target by the agent's current name.
    Name(String),
}

impl AgentTarget {
    /// Validate and normalize the selector.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an empty value, an over-long
    /// name, or a pubkey that is not 64 hex characters.
    pub fn validated(&self) -> Result<Self, SdkError> {
        match self {
            Self::Pubkey(pubkey) => {
                let pubkey = required(pubkey, "agent pubkey", 64)?;
                if pubkey.len() != 64 || !pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(SdkError::InvalidInput(
                        "agent pubkey must be 64 hex characters".into(),
                    ));
                }
                Ok(Self::Pubkey(pubkey.to_ascii_lowercase()))
            }
            Self::Name(name) => Ok(Self::Name(required(name, "agent name", MAX_NAME_CHARS)?)),
        }
    }
}

/// Arguments for `agents.create`.
///
/// `channelId` is present because creation genuinely needs it: the new agent is
/// attached to that channel. Update and delete carry no channel, since the
/// target is identified by the agent itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsCreateArgs {
    /// Channel the new agent is attached to.
    pub channel_id: String,
    /// Name for the new agent.
    pub display_name: String,
    /// Instructions the new agent runs with.
    pub system_prompt: String,
    /// Preferred harness id; the host refuses a runtime it cannot resolve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Inference provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model identifier, interpreted relative to the runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Inbound author gate mode; absent = the host's owner-only default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
}

impl AgentsCreateArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID, an
    /// empty or over-long name or prompt, or an unsupported respond-to mode.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
            display_name: required(&self.display_name, "display name", MAX_NAME_CHARS)?,
            system_prompt: required(&self.system_prompt, "system prompt", MAX_PROMPT_CHARS)?,
            runtime: optional(self.runtime.as_ref(), "runtime", MAX_SCALAR_CHARS)?,
            provider: optional(self.provider.as_ref(), "provider", MAX_SCALAR_CHARS)?,
            model: optional(self.model.as_ref(), "model", MAX_SCALAR_CHARS)?,
            respond_to: respond_to(self.respond_to.as_ref())?,
        })
    }
}

/// Arguments for `agents.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsUpdateArgs {
    /// Which agent to patch.
    pub target: AgentTarget,
    /// Rename the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Replacement instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Harness id to pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Inference provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Model identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Inbound author gate mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
}

impl AgentsUpdateArgs {
    /// Validate and normalize, requiring at least one field to change.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed target, an over-long
    /// field, an unsupported respond-to mode, or a request that changes nothing.
    pub fn validated(&self) -> Result<Self, SdkError> {
        let normalized = Self {
            target: self.target.validated()?,
            display_name: optional(self.display_name.as_ref(), "display name", MAX_NAME_CHARS)?,
            system_prompt: optional(
                self.system_prompt.as_ref(),
                "system prompt",
                MAX_PROMPT_CHARS,
            )?,
            runtime: optional(self.runtime.as_ref(), "runtime", MAX_SCALAR_CHARS)?,
            provider: optional(self.provider.as_ref(), "provider", MAX_SCALAR_CHARS)?,
            model: optional(self.model.as_ref(), "model", MAX_SCALAR_CHARS)?,
            respond_to: respond_to(self.respond_to.as_ref())?,
        };
        let unchanged = normalized.display_name.is_none()
            && normalized.system_prompt.is_none()
            && normalized.runtime.is_none()
            && normalized.provider.is_none()
            && normalized.model.is_none()
            && normalized.respond_to.is_none();
        if unchanged {
            return Err(SdkError::InvalidInput(
                "include at least one field to update".into(),
            ));
        }
        Ok(normalized)
    }
}

/// Arguments for `agents.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsDeleteArgs {
    /// Which agent to remove.
    pub target: AgentTarget,
}

impl AgentsDeleteArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed target selector.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            target: self.target.validated()?,
        })
    }
}

/// A capability name paired with its strictly typed arguments.
///
/// Flattened into [`super::BrokerRequest`], so the wire form is
/// `{ "capability": "agents.create", "args": { … } }` and an args shape can
/// never be paired with the wrong capability name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "capability", content = "args")]
pub enum CapabilityArgs {
    /// Mint a managed agent.
    #[serde(rename = "agents.create")]
    AgentsCreate(AgentsCreateArgs),
    /// Patch a managed agent.
    #[serde(rename = "agents.update")]
    AgentsUpdate(AgentsUpdateArgs),
    /// Remove a managed agent.
    #[serde(rename = "agents.delete")]
    AgentsDelete(AgentsDeleteArgs),
}

impl CapabilityArgs {
    /// The capability these args belong to.
    #[must_use]
    pub fn capability(&self) -> Capability {
        match self {
            Self::AgentsCreate(_) => Capability::AgentsCreate,
            Self::AgentsUpdate(_) => Capability::AgentsUpdate,
            Self::AgentsDelete(_) => Capability::AgentsDelete,
        }
    }

    /// Validate the arguments in place.
    ///
    /// # Errors
    ///
    /// Propagates the per-capability validation error.
    pub fn validate(&self) -> Result<(), SdkError> {
        match self {
            Self::AgentsCreate(args) => args.validated().map(|_| ()),
            Self::AgentsUpdate(args) => args.validated().map(|_| ()),
            Self::AgentsDelete(args) => args.validated().map(|_| ()),
        }
    }

    /// Return a normalized copy with every field validated.
    ///
    /// # Errors
    ///
    /// Propagates the per-capability validation error.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(match self {
            Self::AgentsCreate(args) => Self::AgentsCreate(args.validated()?),
            Self::AgentsUpdate(args) => Self::AgentsUpdate(args.validated()?),
            Self::AgentsDelete(args) => Self::AgentsDelete(args.validated()?),
        })
    }
}

/// Outcome of a successful `agents.create`.
///
/// Carries the new agent's **public** key only. There is no field for the
/// minted secret: the nsec stays on the host, and this type could not transport
/// it even if a handler tried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsCreateOutcome {
    /// The new agent's pubkey (hex).
    pub agent_pubkey: String,
    /// The new agent's name as stored.
    pub display_name: String,
    /// Channel the agent was attached to.
    pub channel_id: String,
}

/// Outcome of a successful `agents.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsUpdateOutcome {
    /// The patched agent's pubkey (hex).
    pub agent_pubkey: String,
    /// The agent's name after the update.
    pub display_name: String,
    /// Names of the fields the host actually changed, sorted.
    pub updated_fields: Vec<String>,
}

/// Outcome of a successful `agents.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsDeleteOutcome {
    /// The removed agent's pubkey (hex).
    pub agent_pubkey: String,
    /// The removed agent's name.
    pub display_name: String,
}

/// A capability-specific success payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "capability", content = "outcome")]
pub enum CapabilityOutcome {
    /// `agents.create` succeeded.
    #[serde(rename = "agents.create")]
    AgentsCreate(AgentsCreateOutcome),
    /// `agents.update` succeeded.
    #[serde(rename = "agents.update")]
    AgentsUpdate(AgentsUpdateOutcome),
    /// `agents.delete` succeeded.
    #[serde(rename = "agents.delete")]
    AgentsDelete(AgentsDeleteOutcome),
}

impl CapabilityOutcome {
    /// The capability that produced this outcome.
    #[must_use]
    pub fn capability(&self) -> Capability {
        match self {
            Self::AgentsCreate(_) => Capability::AgentsCreate,
            Self::AgentsUpdate(_) => Capability::AgentsUpdate,
            Self::AgentsDelete(_) => Capability::AgentsDelete,
        }
    }
}

// ── Shared validators ───────────────────────────────────────────────────────

fn required(value: &str, label: &str, max: usize) -> Result<String, SdkError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(SdkError::InvalidInput(format!("{label} must not be empty")));
    }
    if value.chars().count() > max {
        return Err(SdkError::InvalidInput(format!(
            "{label} is too long (max {max} characters)"
        )));
    }
    Ok(value.to_owned())
}

fn optional(value: Option<&String>, label: &str, max: usize) -> Result<Option<String>, SdkError> {
    value.map(|value| required(value, label, max)).transpose()
}

fn channel(value: &str) -> Result<String, SdkError> {
    let value = required(value, "channel", 128)?;
    uuid::Uuid::parse_str(&value)
        .map_err(|_| SdkError::InvalidInput(format!("invalid channel UUID: {value}")))?;
    Ok(value)
}

fn respond_to(value: Option<&String>) -> Result<Option<String>, SdkError> {
    let value = optional(value, "respond-to", MAX_SCALAR_CHARS)?;
    if let Some(mode) = value.as_deref() {
        if !RESPOND_TO_MODES.contains(&mode) {
            return Err(SdkError::InvalidInput(format!(
                "respond-to must be one of {}",
                RESPOND_TO_MODES.join(", ")
            )));
        }
    }
    Ok(value)
}
