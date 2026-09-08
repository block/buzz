//! Capability-gated provider registration and NIP-OA activation.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        find_managed_agent_mut, load_managed_agents, provider_attest, provider_capabilities,
        provider_register, resolve_provider_binary, save_managed_agents, AgentKeyCustody,
        ProviderRegistration,
    },
    relay::ScopedWorkspaceRelay,
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

/// Proof that the create command validated a freshly negotiated custody mode.
///
/// The field is private so callers cannot construct this token without passing
/// through [`require_expected_custody`]. The create command must consume the
/// token to choose its key-minting or provider-registration path, which binds
/// the guard to that production side-effect boundary at compile time.
#[must_use = "validated custody must select the agent creation path"]
pub(super) struct ValidatedCustodyMode {
    uses_registration: bool,
}

impl ValidatedCustodyMode {
    pub(super) fn uses_registration(&self) -> bool {
        self.uses_registration
    }
}

/// Bind creation to the custody mode the user saw after the provider probe.
/// A fresh negotiation may confirm that mode, but it must never silently
/// switch paths after the UI has described which process receives the key.
pub(super) fn require_expected_custody(
    expected: Option<AgentKeyCustody>,
    uses_registration: bool,
) -> Result<ValidatedCustodyMode, String> {
    let expected = expected.ok_or_else(|| {
        "provider creation requires the key custody observed during provider selection".to_string()
    })?;
    let negotiated = if uses_registration {
        AgentKeyCustody::Provider
    } else {
        AgentKeyCustody::Local
    };
    if negotiated == expected {
        Ok(ValidatedCustodyMode { uses_registration })
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

fn attestation_agent(
    pubkey: &str,
    auth_tag: &str,
    community_relay: &ScopedWorkspaceRelay,
) -> serde_json::Value {
    serde_json::json!({
        "pubkey": pubkey,
        "auth_tag": auth_tag,
        "community_url": community_relay.as_str(),
    })
}

fn attestation_pending_after(was_pending: bool, succeeded: bool) -> bool {
    was_pending && !succeeded
}

/// Attest the saved record in one community and persist a visible error if
/// activation or enrollment fails.
///
/// The provider operation is deliberately idempotent: creation uses it for the
/// first community, and every later community-scoped start repeats it so the
/// same provider-custodied identity can enroll wherever its owner is a member.
pub(super) async fn attest(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    provider_id: &str,
    provider_config: &serde_json::Value,
    community_relay: &ScopedWorkspaceRelay,
) -> Result<(), String> {
    let (auth_tag, was_pending) = {
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        (
            record
                .auth_tag
                .clone()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("agent {pubkey} has no auth tag"))?,
            record.provider_attestation_pending,
        )
    };

    let binary = resolve_provider_binary(provider_id)?;
    let config = provider_config.clone();
    let agent = attestation_agent(pubkey, &auth_tag, community_relay);
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
    // A failed first attest leaves activation pending. A later community's
    // enrollment failure must not make an already activated agent globally
    // undeployed; the scoped Start call still returns the failure to its caller.
    record.provider_attestation_pending = attestation_pending_after(was_pending, result.is_ok());
    save_managed_agents(app, &records)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::bind_expected_relay_scope;

    #[test]
    fn attestation_payload_binds_the_target_community() {
        let community = bind_expected_relay_scope(
            Some("wss://community.example"),
            "wss://community.example".to_string(),
        )
        .unwrap();

        assert_eq!(
            attestation_agent("agent-pubkey", "owner-auth", &community),
            serde_json::json!({
                "pubkey": "agent-pubkey",
                "auth_tag": "owner-auth",
                "community_url": "wss://community.example",
            })
        );
    }

    #[test]
    fn later_community_failure_does_not_reopen_global_attestation() {
        assert!(attestation_pending_after(true, false));
        assert!(!attestation_pending_after(true, true));
        assert!(!attestation_pending_after(false, false));
        assert!(!attestation_pending_after(false, true));
    }

    #[test]
    fn custody_validation_returns_the_command_path_token_only_for_an_exact_match() {
        assert!(
            require_expected_custody(Some(AgentKeyCustody::Provider), true)
                .unwrap()
                .uses_registration()
        );
        assert!(
            !require_expected_custody(Some(AgentKeyCustody::Local), false)
                .unwrap()
                .uses_registration()
        );
        assert!(require_expected_custody(Some(AgentKeyCustody::Provider), false).is_err());
        assert!(require_expected_custody(Some(AgentKeyCustody::Local), true).is_err());
        assert!(require_expected_custody(None, true).is_err());
    }
}
