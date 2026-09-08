//! Safe owner-private projections, not another configuration store or launch plan.
use serde::{Deserialize, Serialize};

/// Exact destination-local configuration version. Edits produce a new revision.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfigurationRef {
    /// Stable configuration identity within the targeted agent and host.
    pub id: String,
    /// Immutable revision selected by the requester, never a latest alias.
    pub revision: String,
}
impl RuntimeConfigurationRef {
    /// Reject malformed references before resolving any local settings.
    pub fn validate(&self) -> Result<(), String> {
        if uuid::Uuid::parse_str(&self.id).is_err()
            || uuid::Uuid::parse_str(&self.revision).is_err()
        {
            return Err("Invalid runtime configuration reference".into());
        }
        Ok(())
    }
}

/// Explicit allowlist for discovery. Never serialize the underlying settings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfigurationSummary {
    /// Exact configuration version checked by the destination.
    pub configuration: RuntimeConfigurationRef,
    /// User-authored display name.
    pub name: String,
    /// Destination Desktop ID, not a hostname or filesystem path.
    pub host: String,
    /// Runtime catalog identifier.
    pub runtime: String,
    /// Deliberately selected model identifier.
    pub model: String,
    /// Deliberately selected provider identifier, never credentials.
    pub provider: Option<String>,
    /// Positive destination-local readiness; unknown must be false.
    pub eligible: bool,
}
impl RuntimeConfigurationSummary {
    /// Bound untrusted display metadata and bind it to the probed host.
    pub fn validate(&self, host: &str) -> Result<(), String> {
        self.configuration.validate()?;
        let text = |value: &str, max| {
            !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        };
        if self.host != host
            || !super::hex(host, 32)
            || !text(&self.name, 120)
            || !text(&self.runtime, 128)
            || !text(&self.model, 512)
            || self.provider.as_ref().is_some_and(|p| !text(p, 128))
        {
            return Err("Invalid runtime configuration summary".into());
        }
        Ok(())
    }
}
