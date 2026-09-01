//! Provider construction.
//!
//! Almost all of this is goose's: `goose::providers::create` resolves base
//! URLs, credentials and model metadata from its own registry, and buzz does
//! not duplicate any of it.
//!
//! # Databricks is the exception, on purpose
//!
//! Both sides implement OAuth 2.0 Authorization Code + PKCE against the same
//! Databricks host, and before this module they did it with **two separate
//! token caches**:
//!
//! | | flow | cache |
//! |---|---|---|
//! | inference | goose `providers::oauth` | goose's config dir |
//! | model picker | [`buzz_model_catalog::auth`] | `~/.config/buzz-agent/oauth/databricks/` |
//!
//! A user could be authenticated for inference and not for the model picker,
//! or the reverse, and be sent through the browser twice for the same host.
//!
//! Buzz's engine is the hardened one — PR #5534 gave it `O_NOFOLLOW`, mode
//! `0600` from creation and unique temp names — and it is also the one the
//! desktop calls directly, since the desktop cannot link goose at all (see
//! `buzz-model-catalog`'s lib.rs). So buzz's engine wins and goose's provider
//! is constructed around it.
//!
//! goose supports this without a fork: `DatabricksV2Provider::new` takes a
//! `DatabricksOauthTokenProvider` closure, and `databricks_v2_def::from_env`
//! is simply the variant that passes goose's own. Buzz passes its own instead.
//! Everything else about the provider — retries, the session-id header,
//! endpoint resolution, the wire format — stays goose's.

use std::sync::Arc;

use goose_providers::base::Provider;
use goose_providers::databricks_auth::{
    DatabricksAuth, DatabricksOauthTokenProvider, DatabricksRefreshHook,
};
use goose_providers::databricks_v2::DatabricksV2Provider;

use crate::types::AgentError;

/// goose's registry name for the v2 Databricks provider.
const DATABRICKS_V2: &str = "databricks_v2";

/// Construct the goose provider for `provider_name`.
///
/// Relay-mesh `auto` is resolved by the desktop before spawn
/// (`managed_agents/relay_mesh.rs:relay_mesh_wire_model`), so buzz-agent sends
/// whatever model it was given and does not know meshes exist (#5289).
pub async fn build(provider_name: &str) -> Result<Arc<dyn Provider>, AgentError> {
    if provider_name == DATABRICKS_V2 {
        if let Some(host) = databricks_host() {
            // Only when buzz has a host to authenticate against. Without one
            // there is nothing to improve on, and falling through to goose's
            // registry keeps `DATABRICKS_TOKEN`-only setups working unchanged.
            return databricks_v2_with_buzz_oauth(host);
        }
    }

    goose::providers::create(provider_name, Vec::new())
        .await
        .map_err(|e| crate::map_provider_error(&e.to_string()))
}

/// The Databricks workspace host, from the environment.
///
/// Read from the environment rather than from goose's `Config` because
/// `Config::get_secret` consults the OS keyring, and an agent subprocess is
/// launched with the environment the desktop gives it.
fn databricks_host() -> Option<String> {
    std::env::var("DATABRICKS_HOST")
        .ok()
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
}

/// goose's `databricks_v2` provider, backed by buzz's PKCE token source.
fn databricks_v2_with_buzz_oauth(host: String) -> Result<Arc<dyn Provider>, AgentError> {
    // A static token still wins, exactly as in goose's own `from_env`: a
    // configured `DATABRICKS_TOKEN` is an explicit instruction not to run an
    // interactive flow at all.
    let auth = match std::env::var("DATABRICKS_TOKEN") {
        Ok(token) if !token.trim().is_empty() => DatabricksAuth::token(token),
        _ => DatabricksAuth::oauth(host.clone()),
    };

    let oauth_token_source = match &auth {
        DatabricksAuth::OAuth { .. } => Some(
            buzz_model_catalog::auth::PkceOAuthTokenSource::new(
                buzz_model_catalog::databricks_pkce_config(&host),
            )
            .map_err(|e| AgentError::Llm(format!("databricks oauth: {e}")))?,
        ),
        DatabricksAuth::Token(_) => None,
    };
    let oauth_token_provider = oauth_token_source
        .as_ref()
        .map(|source| buzz_oauth_token_provider(Arc::clone(source)));
    let refresh_hook = oauth_token_source.map(buzz_oauth_refresh_hook);

    let retry_config = DatabricksV2Provider::load_retry_config(|key| std::env::var(key).ok());

    let provider = DatabricksV2Provider::new(
        host,
        auth,
        retry_config,
        None,
        oauth_token_provider,
        Some(Arc::new(|| {
            std::env::var("DATABRICKS_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty())
        })),
        None,
        refresh_hook,
    )
    .map_err(|e| AgentError::Llm(format!("databricks provider: {e}")))?;

    Ok(Arc::new(provider))
}

/// Adapts buzz's [`TokenSource`](buzz_model_catalog::auth::TokenSource) to the
/// closure shape goose's Databricks provider asks for.
///
/// `bearer_no_browser` rather than `bearer`: this runs inside an inference
/// request on a headless agent subprocess, where the browser step would hang
/// with nobody to see it. Interactive login stays where a human can answer it
/// — `buzz-agent auth databricks`, and the desktop model picker.
fn buzz_oauth_token_provider(
    source: Arc<buzz_model_catalog::auth::PkceOAuthTokenSource>,
) -> DatabricksOauthTokenProvider {
    Arc::new(move |_host, _client_id, _redirect_url, _scopes| {
        let source = Arc::clone(&source);
        Box::pin(async move {
            use buzz_model_catalog::auth::TokenSource;
            source
                .bearer_no_browser()
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
        })
    })
}

/// Goose's refresh hook is synchronous, so mark the rejected access token in
/// shared state and let the next async token-provider call force-refresh it.
/// This keeps the refresh grant headless and avoids blocking a runtime worker.
fn buzz_oauth_refresh_hook(
    source: Arc<buzz_model_catalog::auth::PkceOAuthTokenSource>,
) -> DatabricksRefreshHook {
    Arc::new(move || source.reject_current_bearer())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards against the environment leaking between these tests and the rest
    /// of the suite: they mutate process-global state.
    fn with_env<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let previous: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(key, _)| (key.to_string(), std::env::var(key).ok()))
            .collect();
        for (key, value) in vars {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let result = f();
        for (key, value) in previous {
            match value {
                Some(value) => std::env::set_var(&key, value),
                None => std::env::remove_var(&key),
            }
        }
        result
    }

    /// One test, not three, because `std::env` is process-global and Rust
    /// runs tests in this binary on parallel threads: separate tests touching
    /// `DATABRICKS_HOST` would race each other and fail intermittently.
    #[test]
    fn the_host_is_read_and_trimmed_and_blank_means_absent() {
        with_env(
            &[("DATABRICKS_HOST", Some(" https://example.databricks.com "))],
            || {
                assert_eq!(
                    databricks_host().as_deref(),
                    Some("https://example.databricks.com")
                );
            },
        );
        // An empty or whitespace host is what a desktop launch with an
        // unconfigured workspace produces. Treating it as present would build
        // a provider pointed at "", failing later with a confusing URL error
        // instead of falling back to goose's registry cleanly.
        for blank in ["", "   "] {
            with_env(&[("DATABRICKS_HOST", Some(blank))], || {
                assert!(databricks_host().is_none(), "blank host {blank:?} accepted");
            });
        }
        with_env(&[("DATABRICKS_HOST", None)], || {
            assert!(databricks_host().is_none());
        });
    }

    /// Also one test: same process-global environment, same race otherwise.
    #[test]
    fn the_provider_builds_with_a_token_and_without_one() {
        let host = "https://example.databricks.com";
        // A configured token is an explicit instruction not to run any
        // interactive flow, and goose's own `from_env` honours it. buzz must
        // not quietly change that.
        with_env(
            &[
                ("DATABRICKS_HOST", Some(host)),
                ("DATABRICKS_TOKEN", Some("dapi-secret")),
            ],
            || assert!(databricks_v2_with_buzz_oauth(host.to_string()).is_ok()),
        );
        // A blank token is not a token. Treating it as one would send
        // `Authorization: Bearer ` and 401 every request, never trying the
        // cached OAuth credential that would have worked.
        with_env(
            &[
                ("DATABRICKS_HOST", Some(host)),
                ("DATABRICKS_TOKEN", Some("  ")),
            ],
            || assert!(databricks_v2_with_buzz_oauth(host.to_string()).is_ok()),
        );
        with_env(
            &[("DATABRICKS_HOST", Some(host)), ("DATABRICKS_TOKEN", None)],
            || assert!(databricks_v2_with_buzz_oauth(host.to_string()).is_ok()),
        );
    }
}
