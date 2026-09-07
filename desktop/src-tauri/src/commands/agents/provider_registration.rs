//! Capability-gated provider registration and NIP-OA activation.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        find_managed_agent_mut, load_managed_agents, provider_attest, provider_capabilities,
        provider_register, resolve_provider_binary, save_managed_agents, AgentKeyCustody,
        ProviderRegistration,
    },
    util::now_iso,
};

/// A provider owns identity creation only when it explicitly advertises both
/// halves of the handshake. Providers with neither capability retain the v1
/// deploy flow; advertising only one half fails closed as a protocol error.
pub(super) async fn uses_registration(provider_id: &str) -> Result<bool, String> {
    let binary = resolve_provider_binary(provider_id)?;
    let capabilities = tokio::task::spawn_blocking(move || provider_capabilities(&binary))
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))??;
    let register = capabilities.iter().any(|value| value == "register");
    let attest = capabilities.iter().any(|value| value == "attest");
    match (register, attest) {
        (true, true) => Ok(true),
        (false, false) => Ok(false),
        _ => Err(
            "provider must advertise register and attest together to manage agent identity"
                .to_string(),
        ),
    }
}

/// Bind creation to the custody mode the user saw after the provider probe.
/// A fresh negotiation may confirm that mode, but it must never silently
/// switch paths after the UI has described which process receives the key.
pub(super) fn require_expected_custody(
    expected: AgentKeyCustody,
    uses_registration: bool,
) -> Result<(), String> {
    let negotiated = if uses_registration {
        AgentKeyCustody::Provider
    } else {
        AgentKeyCustody::Local
    };
    if negotiated == expected {
        Ok(())
    } else {
        Err(format!(
            "provider key custody changed after selection (expected {expected:?}, negotiated {negotiated:?}); select the provider again"
        ))
    }
}

pub(super) async fn register(
    provider_id: &str,
    provider_config: &serde_json::Value,
    name: &str,
) -> Result<ProviderRegistration, String> {
    let binary = resolve_provider_binary(provider_id)?;
    let config = provider_config.clone();
    let agent = serde_json::json!({ "name": name });
    tokio::task::spawn_blocking(move || provider_register(&binary, &agent, &config))
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
        .map_err(|error| format!("provider register failed: {error}"))
}

/// Attest the saved record and persist a visible error if activation fails.
pub(super) async fn attest(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    provider_id: &str,
    provider_config: &serde_json::Value,
) -> Result<(), String> {
    let auth_tag = {
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        record
            .auth_tag
            .clone()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("agent {pubkey} has no auth tag"))?
    };

    let binary = resolve_provider_binary(provider_id)?;
    let config = provider_config.clone();
    let agent = serde_json::json!({ "pubkey": pubkey, "auth_tag": auth_tag });
    let result = tokio::task::spawn_blocking(move || provider_attest(&binary, &agent, &config))
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
        .map_err(|error| format!("provider attest failed: {error}"));

    let _guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.updated_at = now_iso();
    record.last_error = result.as_ref().err().cloned();
    record.provider_attestation_pending = result.is_err();
    save_managed_agents(app, &records)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custody_assertion_fails_closed_when_negotiation_changes() {
        assert!(require_expected_custody(AgentKeyCustody::Provider, true).is_ok());
        assert!(require_expected_custody(AgentKeyCustody::Local, false).is_ok());
        assert!(require_expected_custody(AgentKeyCustody::Provider, false).is_err());
        assert!(require_expected_custody(AgentKeyCustody::Local, true).is_err());
    }
}
