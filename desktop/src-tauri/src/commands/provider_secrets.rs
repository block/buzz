//! Device-scoped provider credentials.
//!
//! Only credentials explicitly marked `device_keyring` by the Rust provider
//! catalog may use this surface. Commands return presence/source metadata and
//! never return credential values.

use serde::Serialize;
use tauri::AppHandle;

use crate::secret_store::{KeyringProbe, SecretStore};

const PROVIDER_SECRET_PREFIX: &str = "provider-secret:";
const MAX_PROVIDER_SECRET_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSecretSource {
    /// A non-empty process environment value takes precedence.
    Environment,
    /// The device keyring contains the credential.
    Keyring,
    /// Secure storage is reachable but the credential is absent.
    Missing,
    /// Secure storage could not be reached.
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSecretStatus {
    /// Canonical provider identifier.
    pub provider_id: String,
    /// Credential environment-key identifier.
    pub credential_id: String,
    /// Whether the effective environment or keyring supplies a credential.
    pub configured: bool,
    /// Source that determined `configured`.
    pub source: ProviderSecretSource,
    /// Running local agents restarted with the updated credential.
    pub restarted_count: u32,
    /// Agents stopped successfully but not restarted successfully.
    pub failed_restart_count: u32,
}

fn device_credential(provider_id: &str) -> Result<(&'static str, &'static str), String> {
    let profile = buzz_agent_pkg::provider_profiles::provider_profile(provider_id)
        .ok_or_else(|| format!("unknown provider {provider_id:?}"))?;
    let credential = profile
        .credential
        .filter(|credential| credential.device_keyring)
        .ok_or_else(|| format!("provider {:?} has no device credential", profile.id))?;
    Ok((profile.id, credential.env))
}

fn keyring_key(credential_id: &str) -> String {
    format!("{PROVIDER_SECRET_PREFIX}{credential_id}")
}

fn provider_secret_store() -> &'static SecretStore {
    SecretStore::shared(crate::app_state::keyring_service())
}

pub(crate) fn load_provider_secret(provider_id: &str) -> Result<Option<String>, String> {
    let (_, credential_id) = device_credential(provider_id)?;
    if let Ok(value) = std::env::var(credential_id) {
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    provider_secret_store().load(&keyring_key(credential_id))
}

fn provider_secret_status_with(
    provider_id: &str,
    env_value: Option<&str>,
    probe: impl FnOnce(&str) -> KeyringProbe,
) -> Result<ProviderSecretStatus, String> {
    let (canonical_id, credential_id) = device_credential(provider_id)?;
    let source = if env_value.is_some_and(|value| !value.trim().is_empty()) {
        ProviderSecretSource::Environment
    } else {
        match probe(&keyring_key(credential_id)) {
            KeyringProbe::Present => ProviderSecretSource::Keyring,
            KeyringProbe::ReachableButEmpty => ProviderSecretSource::Missing,
            KeyringProbe::Unreachable => ProviderSecretSource::Unavailable,
        }
    };
    Ok(ProviderSecretStatus {
        provider_id: canonical_id.to_string(),
        credential_id: credential_id.to_string(),
        configured: matches!(
            source,
            ProviderSecretSource::Environment | ProviderSecretSource::Keyring
        ),
        source,
        restarted_count: 0,
        failed_restart_count: 0,
    })
}

#[tauri::command]
/// Return effective device-credential presence without returning its value.
pub fn get_provider_secret_status(provider_id: String) -> Result<ProviderSecretStatus, String> {
    let (_, credential_id) = device_credential(&provider_id)?;
    let env_value = std::env::var(credential_id).ok();
    provider_secret_status_with(&provider_id, env_value.as_deref(), |key| {
        provider_secret_store().probe(key)
    })
}

#[tauri::command]
/// Save a catalog-authorized provider credential in the OS keyring.
pub async fn set_provider_secret(
    provider_id: String,
    value: String,
    app: AppHandle,
) -> Result<ProviderSecretStatus, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("provider credential cannot be empty".to_string());
    }
    if value.len() > MAX_PROVIDER_SECRET_BYTES {
        return Err(format!(
            "provider credential exceeds {MAX_PROVIDER_SECRET_BYTES} bytes"
        ));
    }
    if value.contains('\0') {
        return Err("provider credential cannot contain NUL bytes".to_string());
    }
    let (_, credential_id) = device_credential(&provider_id)?;
    let old_value = load_provider_secret(&provider_id)?;
    provider_secret_store().store(&keyring_key(credential_id), value)?;
    let new_value = load_provider_secret(&provider_id)?;
    let (restarted_count, failed_restart_count) =
        super::restart_local_agents_for_provider_secret_change(
            &app,
            &provider_id,
            credential_id,
            old_value.as_deref(),
            new_value.as_deref(),
        )
        .await;
    let mut status = get_provider_secret_status(provider_id)?;
    status.restarted_count = restarted_count;
    status.failed_restart_count = failed_restart_count;
    Ok(status)
}

#[tauri::command]
/// Remove a catalog-authorized provider credential from the OS keyring.
pub async fn clear_provider_secret(
    provider_id: String,
    app: AppHandle,
) -> Result<ProviderSecretStatus, String> {
    let (_, credential_id) = device_credential(&provider_id)?;
    let old_value = load_provider_secret(&provider_id)?;
    provider_secret_store().delete(&keyring_key(credential_id))?;
    let new_value = load_provider_secret(&provider_id)?;
    let (restarted_count, failed_restart_count) =
        super::restart_local_agents_for_provider_secret_change(
            &app,
            &provider_id,
            credential_id,
            old_value.as_deref(),
            new_value.as_deref(),
        )
        .await;
    let mut status = get_provider_secret_status(provider_id)?;
    status.restarted_count = restarted_count;
    status.failed_restart_count = failed_restart_count;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_never_contains_secret_value() {
        let status = provider_secret_status_with("huggingface", Some("hf_secret"), |_| {
            KeyringProbe::ReachableButEmpty
        })
        .expect("status");
        assert_eq!(status.source, ProviderSecretSource::Environment);
        assert!(status.configured);
        let serialized = serde_json::to_string(&status).expect("serialize");
        assert!(!serialized.contains("hf_secret"));
    }

    #[test]
    fn only_catalog_device_credentials_are_accepted() {
        assert_eq!(device_credential("hf"), Ok(("huggingface", "HF_TOKEN")));
        assert!(device_credential("openai").is_err());
        assert!(device_credential("ollama").is_err());
        assert!(device_credential("unknown").is_err());
    }

    #[test]
    fn keyring_probe_is_projected_without_loading_a_value() {
        let present = provider_secret_status_with("huggingface", None, |_| KeyringProbe::Present)
            .expect("present");
        assert_eq!(present.source, ProviderSecretSource::Keyring);
        let missing =
            provider_secret_status_with("huggingface", None, |_| KeyringProbe::ReachableButEmpty)
                .expect("missing");
        assert_eq!(missing.source, ProviderSecretSource::Missing);
        assert!(!missing.configured);
    }
}
