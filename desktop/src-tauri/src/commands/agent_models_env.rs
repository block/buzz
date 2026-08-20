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

/// Read a trimmed mapped value even when it is blank, falling back to the
/// process only when the map has no override. Optional credentials use this so
/// an explicit blank means "send no authentication" rather than inheriting an
/// unrelated process secret.
pub(super) fn env_or_process_override(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(|value| value.trim().to_string())
        .or_else(|| {
            std::env::var(key)
                .ok()
                .map(|value| value.trim().to_string())
        })
}

pub(super) fn validate_openai_compat_base_url(value: &str) -> Result<(), String> {
    const INVALID_URL: &str =
        "OPENAI_COMPAT_BASE_URL must be an HTTP(S) URL without credentials, query, or fragment";
    let parsed = url::Url::parse(value.trim()).map_err(|_| INVALID_URL.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(INVALID_URL.to_string());
    }
    Ok(())
}

pub(super) fn openai_compatible_models_url_for_discovery(
    provider: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<String, String> {
    let is_compat =
        provider.is_some_and(|value| value.trim().eq_ignore_ascii_case("openai-compat"));
    let configured_base_url = if is_compat {
        env_or_process_override(env, "OPENAI_COMPAT_BASE_URL")
    } else {
        env_value(env, "OPENAI_COMPAT_BASE_URL")
    }
    .filter(|value| !value.is_empty());
    if !is_compat {
        if configured_base_url
            .as_deref()
            .is_some_and(|value| value.trim_end_matches('/') != "https://api.openai.com/v1")
        {
            return Err(
                "OPENAI_COMPAT_BASE_URL is only supported by provider=openai-compat".into(),
            );
        }
        return Ok("https://api.openai.com/v1/models".to_string());
    }

    let base_url = configured_base_url.ok_or_else(|| {
        "OPENAI_COMPAT_BASE_URL required for OpenAI-compatible model discovery".to_string()
    })?;
    validate_openai_compat_base_url(&base_url)?;
    Ok(format!("{}/models", base_url.trim().trim_end_matches('/')))
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
    pub(super) value: Option<String>,
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
        match env_or_process_override(env, key).filter(|value| !value.is_empty()) {
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

#[cfg(test)]
mod tests {
    use super::validate_openai_compat_base_url;

    #[test]
    fn rejects_ambiguous_openai_compat_url_components() {
        for value in [
            "http://user:secret@localhost/v1",
            "http://localhost/v1?tenant=x",
            "http://localhost/v1#fragment",
        ] {
            let error = validate_openai_compat_base_url(value).unwrap_err();
            assert!(
                error.contains("without credentials, query, or fragment"),
                "value={value} error={error}"
            );
        }
    }
}
