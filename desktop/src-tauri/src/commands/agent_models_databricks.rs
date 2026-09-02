//! Databricks v1/v2 model discovery and interactive reauthentication.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::commands::agent_models_env::{
    env_or_process_value, env_value_or_process_if_absent, redaction_env_with_value,
    DiscoveryProvider,
};
use crate::managed_agents::AgentModelInfo;
use crate::managed_agents::AgentModelsResponse;

// Model discovery can be triggered by multiple dialogs at once. Permit only one
// callback listener/browser flow for the process-wide OAuth cache.
static AUTH_GATE: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

// Hard cap on the interactive browser flow launched from a discovery surface.
// An abandoned SSO tab must fail discovery cleanly rather than wedge the
// dropdown forever. (`authenticate_databricks` has its own 60s callback wait;
// this outer bound also covers endpoint discovery and token exchange.)
const AUTH_FLOW_TIMEOUT: Duration = Duration::from_secs(150);

// How long a failed/cancelled interactive sign-in suppresses re-launching the
// browser from passive surfaces.
pub(super) const AUTH_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Per-host record of a recently failed, cancelled, or timed-out interactive
/// sign-in.
///
/// Passive discovery surfaces fire on every form-state change, so without this
/// a cancelled SSO page would re-pop the browser on the very next keystroke.
/// Entries expire so a genuine later retry still launches; the saved-model
/// picker bypasses the cooldown and a success clears it.
#[derive(Default)]
pub(super) struct AuthCooldown {
    until: Mutex<HashMap<String, Instant>>,
}

impl AuthCooldown {
    fn map(&self) -> MutexGuard<'_, HashMap<String, Instant>> {
        // The critical sections below are panic-free map ops, so recover from a
        // poisoned lock rather than wedge every future sign-in on one panic.
        self.until
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn is_active(&self, host: &str, now: Instant) -> bool {
        let mut map = self.map();
        match map.get(host) {
            Some(&expiry) if now < expiry => true,
            Some(_) => {
                map.remove(host);
                false
            }
            None => false,
        }
    }

    pub(super) fn record(&self, host: &str, now: Instant) {
        self.map().insert(host.to_string(), now + AUTH_COOLDOWN);
    }

    pub(super) fn clear(&self, host: &str) {
        self.map().remove(host);
    }

    /// Whether the interactive browser flow may launch now under `auth_intent`.
    /// Passive surfaces are suppressed while a per-host cooldown is active; the
    /// explicit picker path always launches and clears any stale suppression.
    pub(super) fn permits_launch(
        &self,
        auth_intent: DatabricksAuthIntent,
        host: &str,
        now: Instant,
    ) -> bool {
        if auth_intent.respects_cooldown() {
            !self.is_active(host, now)
        } else {
            self.clear(host);
            true
        }
    }
}

static AUTH_COOLDOWNS: LazyLock<AuthCooldown> = LazyLock::new(AuthCooldown::default);

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
    /// Passive draft discovery honors (and, on failure, writes) the per-host
    /// cooldown so a cancelled SSO page does not re-pop on the next form
    /// keystroke. The saved-model picker is an explicit user action, so it
    /// bypasses the cooldown and clears it before launching. Both surfaces
    /// launch the browser flow (Phase 2 goose-parity); this predicate is the
    /// only behavioral difference between them.
    fn respects_cooldown(self) -> bool {
        matches!(self, Self::PassiveDraftDiscovery)
    }
}

pub(super) fn databricks_sign_in_required_error() -> String {
    "Databricks sign-in is required; save this agent, then open its model picker to sign in, or run `buzz-agent auth databricks`"
        .to_string()
}

pub(super) fn databricks_sign_in_timed_out_error() -> String {
    "Databricks sign-in timed out; open the model picker to retry, or run `buzz-agent auth databricks`"
        .to_string()
}

pub(super) fn should_start_interactive_auth(api_key: &str) -> bool {
    // Phase 2: both discovery surfaces launch the browser flow when no static
    // token is configured. Which surface is allowed to actually pop the browser
    // (vs. respect a cooldown) is decided via `AuthCooldown::permits_launch`.
    api_key.is_empty()
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
    let filter = env_value_or_process_if_absent(env, "DATABRICKS_MODEL_FILTER");
    let parsed_filter = buzz_agent_pkg::config::DatabricksModelFilter::parse(filter.as_deref())
        .map_err(|error| format!("invalid DATABRICKS_MODEL_FILTER: {error}"))?;
    let config = buzz_agent_pkg::config::Config::for_discovery(
        databricks_agent_provider(provider_name),
        api_key.clone(),
        host.clone(),
        parsed_filter.clone(),
    );
    let redaction_env = redaction_env_with_value(env, "DATABRICKS_TOKEN", &api_key);
    let oauth_cache_dir = crate::build_identity::demo_agent_oauth_cache_dir()?;

    let entries = match buzz_agent_pkg::discover_databricks_models_with_cache_dir(
        &config,
        oauth_cache_dir.as_deref(),
    )
    .await
    {
        Ok(entries) => entries,
        Err(buzz_agent_pkg::AgentError::LlmAuth(_)) if should_start_interactive_auth(&api_key) => {
            let _auth = AUTH_GATE.lock().await;
            match buzz_agent_pkg::discover_databricks_models_with_cache_dir(
                &config,
                oauth_cache_dir.as_deref(),
            )
            .await
            {
                // A peer sign-in under the gate already succeeded.
                Ok(entries) => entries,
                Err(buzz_agent_pkg::AgentError::LlmAuth(_)) => {
                    // Passive surfaces suppress the browser while a recent
                    // failure/cancel is cooling down; the explicit picker path
                    // always launches (and clears any stale cooldown).
                    if !AUTH_COOLDOWNS.permits_launch(auth_intent, &host, Instant::now()) {
                        return Err(databricks_sign_in_required_error());
                    }
                    run_interactive_databricks_auth(
                        buzz_agent_pkg::authenticate_databricks_with_cache_dir(
                            &host,
                            oauth_cache_dir.as_deref(),
                        ),
                        AUTH_FLOW_TIMEOUT,
                        &AUTH_COOLDOWNS,
                        &host,
                        &redaction_env,
                    )
                    .await?;
                    buzz_agent_pkg::discover_databricks_models_with_cache_dir(
                        &config,
                        oauth_cache_dir.as_deref(),
                    )
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

    databricks_models_response(
        provider_name,
        entries,
        selected_model,
        parsed_filter.as_ref(),
    )
    .map(Some)
}

/// When a catalog query fails, Desktop reports the catalog error to the UI and
/// does not fall through to subprocess discovery, so the filter cannot be
/// bypassed by a second model source.
pub(super) fn databricks_models_response(
    provider_name: &str,
    entries: Vec<buzz_agent_pkg::ModelEntry>,
    selected_model: Option<String>,
    filter: Option<&buzz_agent_pkg::config::DatabricksModelFilter>,
) -> Result<AgentModelsResponse, String> {
    let entries_are_empty = entries.is_empty();
    if entries_are_empty && filter.is_none() {
        return Err("Databricks model discovery returned no models".to_string());
    }

    Ok(AgentModelsResponse {
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
        supports_switching: !entries_are_empty,
    })
}

fn format_redacted_error(
    context: &str,
    error: &impl std::fmt::Display,
    redaction_env: &BTreeMap<String, String>,
) -> String {
    let message = crate::managed_agents::redact_env_values_in(&error.to_string(), redaction_env);
    format!("{context}: {message}")
}

/// Run the interactive browser OAuth flow under a hard timeout and maintain the
/// per-host cooldown. Success clears the cooldown; a failure, cancel, or
/// timeout records it so passive surfaces stop re-launching the browser on the
/// next form keystroke. `timeout` is injected (production passes
/// [`AUTH_FLOW_TIMEOUT`]) so the timeout/cooldown policy is unit-testable
/// without a live browser.
pub(super) async fn run_interactive_databricks_auth<Fut>(
    auth: Fut,
    timeout: Duration,
    cooldowns: &AuthCooldown,
    host: &str,
    redaction_env: &BTreeMap<String, String>,
) -> Result<(), String>
where
    Fut: std::future::Future<Output = Result<(), buzz_agent_pkg::AgentError>>,
{
    match tokio::time::timeout(timeout, auth).await {
        Ok(Ok(())) => {
            cooldowns.clear(host);
            Ok(())
        }
        Ok(Err(error)) => {
            cooldowns.record(host, Instant::now());
            Err(format_redacted_error(
                "Databricks sign-in failed",
                &error,
                redaction_env,
            ))
        }
        Err(_elapsed) => {
            cooldowns.record(host, Instant::now());
            Err(databricks_sign_in_timed_out_error())
        }
    }
}

#[cfg(test)]
#[path = "agent_models_databricks_tests.rs"]
mod tests;
