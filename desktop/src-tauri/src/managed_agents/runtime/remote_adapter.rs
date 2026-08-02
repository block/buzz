use std::process::Command;

use super::super::readiness::EffectiveHarnessDescriptor;

pub(super) fn load_bearer_token(
    descriptor: &EffectiveHarnessDescriptor,
) -> Result<Option<String>, String> {
    if super::super::discovery::normalize_command_identity(&descriptor.command) != "buzz-a2a-acp" {
        return Ok(None);
    }
    match (
        descriptor.env.get("BUZZ_A2A_AGENT_RECORD"),
        descriptor.env.get("BUZZ_A2A_BEARER_ENDPOINT"),
    ) {
        (Some(record_url), Some(endpoint)) => {
            crate::commands::load_remote_agency_bearer_token(record_url, endpoint)
        }
        _ => Ok(None),
    }
}

pub(super) fn apply_bearer_token(command: &mut Command, token: Option<String>) {
    command.env_remove("BUZZ_A2A_BEARER_TOKEN");
    if let Some(token) = token {
        command.env("BUZZ_A2A_BEARER_TOKEN", token);
    }
}
