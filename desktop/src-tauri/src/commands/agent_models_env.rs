//! Env lookups shared by the in-process model discovery paths.
//!
//! Discovery reads configuration from a merged env map (record/form env over the
//! runtime-derived env, under the baked build floor) and falls back to the
//! process env for values a GUI launch never wrote into the map.

use std::collections::BTreeMap;

/// Read a non-blank value from the merged discovery env.
pub(super) fn env_value(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Read a non-blank value from the merged discovery env, then the process env.
pub(super) fn env_or_process_value(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env_value(env, key).or_else(|| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// Read a value from the merged discovery env, preserving an explicit blank
/// override. Only when the merged map has no such key does the inherited
/// process environment provide a fallback. This mirrors the child process,
/// where a merged key overrides the inherited environment even when blank.
pub(super) fn env_value_or_process_if_absent(
    env: &BTreeMap<String, String>,
    key: &str,
) -> Option<String> {
    match env.get(key) {
        Some(value) => Some(value.trim().to_string()),
        None => std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string()),
    }
}

/// Clone `env` with `key` set to the value a request actually used, so error
/// redaction masks the inherited process value and not just the mapped one.
pub(super) fn redaction_env_with_value(
    env: &BTreeMap<String, String>,
    key: &str,
    value: &str,
) -> BTreeMap<String, String> {
    let mut redaction_env = env.clone();
    redaction_env.insert(key.to_string(), value.to_string());
    redaction_env
}

/// A provider resolved for discovery, plus how it was resolved.
///
/// The distinction decides what a missing credential means. An explicit provider
/// is an assertion — the record or the dialog says this agent runs on Anthropic,
/// so a missing `ANTHROPIC_API_KEY` is a real misconfiguration and the user
/// should see it. An inferred provider is only a guess read off the environment,
/// and a wrong guess must not replace a working catalog with an error: a
/// `GOOSE_PROVIDER=anthropic` export (goose's documented way to pick a provider,
/// with the key in goose's own keyring rather than Buzz's env) would otherwise
/// turn the subprocess catalog into `config: ANTHROPIC_API_KEY required`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiscoveryProvider {
    value: Option<String>,
    inferred: bool,
}

impl DiscoveryProvider {
    pub(super) fn as_deref(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Read a credential the provider's discovery cannot run without.
    ///
    /// `Ok(None)` means "not configured, and the provider was only inferred" —
    /// the caller must fall through to the `buzz-acp models` subprocess rather
    /// than surface an error.
    pub(super) fn required_env(
        &self,
        env: &BTreeMap<String, String>,
        key: &str,
    ) -> Result<Option<String>, String> {
        match env_or_process_value(env, key) {
            Some(value) => Ok(Some(value)),
            None if self.inferred => Ok(None),
            None => Err(format!("config: {key} required")),
        }
    }
}

/// Resolve the provider that live model discovery should run against.
///
/// An explicit provider — the agent record's saved provider, or the create/edit
/// dialog's current form value — always wins. When there is none, fall back to
/// the provider the agent will actually launch with: the runtime's provider env
/// var read out of `env`, which by this point already carries the baked build
/// floor (see `discovery_env_with_baked_floor`) and the process env.
///
/// Without this fallback, every provider gate sees `None` for an agent whose
/// record predates provider persistence, so no in-process discovery runs at all
/// — even on an internal build that bakes `BUZZ_AGENT_PROVIDER=databricks_v2`
/// and a `DATABRICKS_HOST`. Discovery then degrades to the `buzz-acp models`
/// subprocess, which on a Databricks failure path surfaces the small
/// known-models fallback catalog instead of the live gateway list —
/// indistinguishable, from the picker's side, from the real thing.
pub(super) fn effective_discovery_provider(
    provider: Option<&str>,
    provider_env_var: Option<&str>,
    env: &BTreeMap<String, String>,
) -> DiscoveryProvider {
    if let Some(explicit) = provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    {
        return DiscoveryProvider {
            value: Some(explicit),
            inferred: false,
        };
    }

    DiscoveryProvider {
        value: provider_env_var.and_then(|key| env_or_process_value(env, key)),
        inferred: true,
    }
}
