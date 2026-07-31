//! Databricks v1/v2 model discovery and interactive reauthentication.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use crate::commands::agent_models_env::{
    env_or_process_value, redaction_env_with_value, DiscoveryProvider,
};
use crate::managed_agents::AgentModelInfo;
use crate::managed_agents::AgentModelsResponse;

// Model discovery can be triggered by multiple dialogs at once. Permit only one
// callback listener/browser flow for the process-wide OAuth cache.
static AUTH_GATE: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

pub(super) fn is_databricks_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("databricks" | "databricks_v2" | "databricks-v2")
    )
}

fn databricks_agent_provider(provider: &str) -> buzz_agent_pkg::config::Provider {
    if provider.trim().eq_ignore_ascii_case("databricks_v2")
        || provider.trim().eq_ignore_ascii_case("databricks-v2")
    {
        buzz_agent_pkg::config::Provider::DatabricksV2
    } else {
        buzz_agent_pkg::config::Provider::Databricks
    }
}

pub(super) fn databricks_static_token_error(
    error: &str,
    redaction_env: &BTreeMap<String, String>,
) -> String {
    let message = crate::managed_agents::redact_env_values_in(error, redaction_env);
    format!("Databricks rejected DATABRICKS_TOKEN; update it in agent settings: {message}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DatabricksAuthIntent {
    /// A saved agent's model picker was opened by the user.
    InteractiveModelPicker,
    /// Discovery was triggered automatically from unsaved form state.
    PassiveDraftDiscovery,
}

impl DatabricksAuthIntent {
    fn allows_interactive_auth(self) -> bool {
        matches!(self, Self::InteractiveModelPicker)
    }
}

pub(super) fn databricks_sign_in_required_error() -> String {
    "Databricks sign-in is required; save this agent, then open its model picker to sign in, or run `buzz-agent auth databricks`"
        .to_string()
}

pub(super) fn should_start_interactive_auth(
    api_key: &str,
    auth_intent: DatabricksAuthIntent,
) -> bool {
    api_key.is_empty() && auth_intent.allows_interactive_auth()
}

pub(super) async fn discover_databricks_models(
    _client: &reqwest::Client,
    provider: &DiscoveryProvider,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
    auth_intent: DatabricksAuthIntent,
) -> Result<Option<AgentModelsResponse>, String> {
    let provider_name = match provider.as_deref() {
        Some(provider_name) if is_databricks_provider(Some(provider_name)) => provider_name,
        _ => return Ok(None),
    };

    let host = match env_or_process_value(env, "DATABRICKS_HOST") {
        Some(host) => host,
        None => return Ok(None),
    };
    let api_key = env_or_process_value(env, "DATABRICKS_TOKEN").unwrap_or_default();
    let config = buzz_agent_pkg::config::Config::for_discovery(
        databricks_agent_provider(provider_name),
        api_key.clone(),
        host.clone(),
    );
    let redaction_env = redaction_env_with_value(env, "DATABRICKS_TOKEN", &api_key);

    let entries = match buzz_agent_pkg::discover_databricks_models(&config).await {
        Ok(entries) => entries,
        Err(buzz_agent_pkg::AgentError::LlmAuth(_))
            if should_start_interactive_auth(&api_key, auth_intent) =>
        {
            let _auth = AUTH_GATE.lock().await;
            match buzz_agent_pkg::discover_databricks_models(&config).await {
                Ok(entries) => entries,
                Err(buzz_agent_pkg::AgentError::LlmAuth(_)) => {
                    buzz_agent_pkg::authenticate_databricks(&host)
                        .await
                        .map_err(|error| {
                            format_redacted_error(
                                "Databricks sign-in failed",
                                &error,
                                &redaction_env,
                            )
                        })?;
                    buzz_agent_pkg::discover_databricks_models(&config)
                        .await
                        .map_err(|error| {
                            format_redacted_error(
                                "Databricks model discovery failed after sign-in",
                                &error,
                                &redaction_env,
                            )
                        })?
                }
                Err(error) => {
                    return Err(format_redacted_error(
                        "Databricks model discovery failed",
                        &error,
                        &redaction_env,
                    ));
                }
            }
        }
        Err(buzz_agent_pkg::AgentError::LlmAuth(error)) if !api_key.is_empty() => {
            return Err(databricks_static_token_error(&error, &redaction_env));
        }
        Err(buzz_agent_pkg::AgentError::LlmAuth(_)) => {
            return Err(databricks_sign_in_required_error());
        }
        Err(error) => {
            return Err(format_redacted_error(
                "Databricks model discovery failed",
                &error,
                &redaction_env,
            ));
        }
    };

    if entries.is_empty() {
        return Err("Databricks model discovery returned no models".to_string());
    }

    Ok(Some(AgentModelsResponse {
        agent_name: provider_name.trim().to_string(),
        agent_version: "models-api".to_string(),
        models: entries
            .into_iter()
            .map(|entry| AgentModelInfo {
                id: entry.id,
                name: Some(entry.name),
                description: None,
            })
            .collect(),
        agent_default_model: None,
        selected_model,
        supports_switching: true,
    }))
}

fn format_redacted_error(
    context: &str,
    error: &impl std::fmt::Display,
    redaction_env: &BTreeMap<String, String>,
) -> String {
    let message = crate::managed_agents::redact_env_values_in(&error.to_string(), redaction_env);
    format!("{context}: {message}")
}
