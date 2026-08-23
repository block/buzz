//! Spawn-boundary environment for catalog-backed Buzz Agent providers.

use std::collections::BTreeMap;
use std::process::Command;

pub(super) fn apply(command: &mut Command, env: &BTreeMap<String, String>) -> Result<(), String> {
    let Some(profile) = env
        .get("BUZZ_AGENT_PROVIDER")
        .and_then(|provider| buzz_agent_pkg::provider_profiles::provider_profile(provider))
    else {
        return Ok(());
    };

    if profile.id == "ollama" {
        crate::ollama::ensure_started_for_agent()?;
        let inherited_base_url = std::env::var("OLLAMA_BASE_URL")
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        if !env.contains_key("OLLAMA_BASE_URL") && !inherited_base_url {
            command.env("OLLAMA_BASE_URL", crate::ollama::openai_base_url()?);
        }
    }
    if let Some(credential) = profile
        .credential
        .filter(|credential| credential.device_keyring)
    {
        if !env.contains_key(credential.env) {
            if let Some(value) = crate::commands::load_provider_secret(profile.id)? {
                command.env(credential.env, value);
            }
        }
    }
    Ok(())
}
