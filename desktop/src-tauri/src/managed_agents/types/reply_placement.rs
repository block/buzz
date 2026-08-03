use super::{validate_respond_to_allowlist, AgentDefinition, ManagedAgentRecord, RespondTo};
use serde::{Deserialize, Serialize};

/// Where a managed agent should place ordinary human-facing replies.
///
/// The enum is intentionally mirrored in `buzz-acp::config` rather than shared
/// across crates so each boundary validates its own input and the desktop can
/// reject malformed persisted records before spawning a child process.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReplyPlacement {
    #[default]
    Thread,
    TopLevel,
    FollowScope,
}

impl ReplyPlacement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::TopLevel => "top-level",
            Self::FollowScope => "follow-scope",
        }
    }

    pub fn parse_wire(value: &str) -> Result<Self, String> {
        match value {
            "thread" => Ok(Self::Thread),
            "top-level" => Ok(Self::TopLevel),
            "follow-scope" => Ok(Self::FollowScope),
            other => Err(format!(
                "reply placement '{other}' is not a recognized mode (expected 'thread', 'top-level', or 'follow-scope')"
            )),
        }
    }
}

/// Resolve the mode that will reach `BUZZ_ACP_REPLY_PLACEMENT`.
pub fn resolve_effective_reply_placement(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global_reply_placement: Option<ReplyPlacement>,
) -> Result<ReplyPlacement, String> {
    if let Some(mode) = record.reply_placement {
        return Ok(mode);
    }

    if let Some(persona_id) = record.persona_id.as_deref() {
        if let Some(persona) = personas.iter().find(|persona| persona.id == persona_id) {
            if let Some(wire) = persona.reply_placement.as_deref() {
                return ReplyPlacement::parse_wire(wire);
            }
        }
    }

    Ok(global_reply_placement.unwrap_or_default())
}

/// The behavioral fields resolved for a new instance at mint time.
#[derive(Debug, PartialEq, Eq)]
pub struct MintBehavioralDefaults {
    pub respond_to: RespondTo,
    pub respond_to_allowlist: Vec<String>,
    pub reply_placement: ReplyPlacement,
    /// Validated (1..=32) when present; caller applies its own default.
    pub parallelism: Option<u32>,
}

/// Resolve the NIP-AP behavioral defaults for a new instance.
pub fn resolve_mint_behavioral_defaults(
    input_respond_to: Option<RespondTo>,
    input_allowlist: Vec<String>,
    input_parallelism: Option<u32>,
    input_reply_placement: Option<ReplyPlacement>,
    definition: Option<&AgentDefinition>,
    global_reply_placement: Option<ReplyPlacement>,
) -> Result<MintBehavioralDefaults, String> {
    let (respond_to, respond_to_allowlist) = match input_respond_to {
        // Explicit instance-level choice: the definition default is ignored
        // wholesale (mode AND list travel together).
        Some(mode) => (mode, input_allowlist),
        None => match definition.and_then(|d| d.respond_to.as_deref()) {
            Some(wire) => {
                let mode = RespondTo::parse_wire(wire)?;
                let list = if input_allowlist.is_empty() {
                    validate_respond_to_allowlist(
                        definition
                            .map(|d| d.respond_to_allowlist.as_slice())
                            .unwrap_or(&[]),
                    )
                    .map_err(|e| format!("definition respond-to allowlist is invalid: {e}"))?
                } else {
                    input_allowlist
                };
                (mode, list)
            }
            None => (RespondTo::default(), input_allowlist),
        },
    };
    if respond_to == RespondTo::Allowlist && respond_to_allowlist.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }

    let reply_placement = match input_reply_placement {
        Some(mode) => mode,
        None => match definition.and_then(|d| d.reply_placement.as_deref()) {
            Some(wire) => ReplyPlacement::parse_wire(wire)?,
            None => global_reply_placement.unwrap_or_default(),
        },
    };

    let parallelism = match input_parallelism {
        // Explicit input is validated here too (not just at the command
        // call sites) so the "validated when present" contract on
        // `MintBehavioralDefaults.parallelism` is unskippable.
        Some(count) if (1..=32).contains(&count) => Some(count),
        Some(count) => {
            return Err(format!(
                "parallelism {count} is out of range (must be between 1 and 32)"
            ))
        }
        None => match definition.and_then(|d| d.parallelism) {
            Some(count) if (1..=32).contains(&count) => Some(count),
            Some(count) => {
                return Err(format!(
                    "parallelism {count} on the linked agent definition is out of range (must be between 1 and 32)"
                ))
            }
            None => None,
        },
    };

    Ok(MintBehavioralDefaults {
        respond_to,
        respond_to_allowlist,
        reply_placement,
        parallelism,
    })
}
