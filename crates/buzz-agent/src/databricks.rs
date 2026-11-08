//! Opt-in, workspace-scoped Databricks OAuth and catalog reuse.
//!
//! Legacy OAuth/catalog entry points retain their destination and intent policy.
//! This entry requires an explicit HTTPS workspace, app-owned cache root and
//! opener. Construction reads only that cache; it performs no network or browser
//! work. The caller must not lend this root to legacy APIs or other applications.

use std::{path::Path, sync::Arc, time::Duration};

use reqwest::{redirect::Policy, Client, ClientBuilder};
use url::Url;

use crate::{
    auth::{AuthError, AuthIntent, BrowserOpener, PkceOAuthTokenSource},
    catalog::{discover_databricks_models_with_client, ModelEntry},
    config::{Config, DatabricksModelFilter, Provider},
    AgentError,
};

/// An explicitly scoped connection, not an agent runtime or a sign-in side effect.
///
/// Only [`connect`](Self::connect) permits browser opening. Discovery is always
/// headless, including its one 401 refresh. Dropping either operation's future
/// cancels its actual HTTP/callback work; callers should impose their own total
/// deadline and fence stale results. Do not hold a controller lock while awaiting.
/// Tokens remain inside the connection; never serialize this object to a webview.
pub struct DatabricksConnection {
    workspace: StrictWorkspace,
    source: Arc<PkceOAuthTokenSource>,
    http: Client,
}

impl DatabricksConnection {
    /// Construct without network/browser work or ambient credential/host lookup.
    ///
    /// `workspace` must be an HTTPS origin (optional trailing slash), without
    /// userinfo, query or fragment. `cache_root` must be absolute and owned by
    /// this app; a separate `databricks-strict` namespace prevents accidental
    /// coalescing with legacy auth. Unix token files retain owner-only atomic
    /// persistence; on non-Unix the existing engine keeps credentials in memory.
    /// The opener must not log the authorization URL or its own error details.
    pub fn new(
        workspace: &str,
        cache_root: &Path,
        opener: Arc<dyn BrowserOpener>,
    ) -> Result<Self, AgentError> {
        Self::build(workspace, cache_root, opener, Client::builder())
    }

    fn build(
        workspace: &str,
        cache_root: &Path,
        opener: Arc<dyn BrowserOpener>,
        http: ClientBuilder,
    ) -> Result<Self, AgentError> {
        let workspace = StrictWorkspace::new(workspace)?;
        if !cache_root.is_absolute() {
            return Err(AgentError::InvalidParams(
                "an absolute app-owned OAuth cache root is required".into(),
            ));
        }
        // Apply policy AFTER the test transport's trust/DNS settings. No caller
        // can inject a client that follows redirects or disable production TLS.
        let http = http
            .redirect(Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| AgentError::Llm("Databricks HTTP client construction failed".into()))?;
        let source = PkceOAuthTokenSource::for_workspace(
            workspace.clone(),
            cache_root,
            opener,
            http.clone(),
        )
        .map_err(|_| AgentError::Llm("Databricks private OAuth cache unavailable".into()))?;
        Ok(Self {
            workspace,
            source,
            http,
        })
    }

    /// Authenticate after an explicit user action. May call the supplied opener.
    /// A valid cached credential short-circuits; this does not fetch models.
    pub async fn connect(&self) -> Result<(), AuthError> {
        self.source
            .acquire_with_intent(AuthIntent::UserInitiated, None)
            .await
            .map(|_| ())
    }

    /// Discover v2 models without a browser, using this connection's workspace
    /// and credentials only. Preserves filtering, partial success and labelled
    /// authenticated-empty/no-filter fallback; this is not proof of completeness.
    /// Does not choose or mutate a model. Errors contain no raw provider content.
    pub async fn discover_models(
        &self,
        filter: Option<DatabricksModelFilter>,
    ) -> Result<Vec<ModelEntry>, AgentError> {
        let cfg = Config::for_discovery(
            Provider::DatabricksV2,
            String::new(),
            self.workspace.as_str().into(),
            filter,
        );
        discover_databricks_models_with_client(&cfg, self.source.clone(), &self.http)
            .await
            .map_err(|error| match error {
                AgentError::LlmAuth(_) => {
                    AgentError::LlmAuth("Databricks authentication required".into())
                }
                _ => AgentError::Llm("Databricks model discovery unavailable".into()),
            })
    }
}

/// Unforgeable outside this module: every production value passed to auth was
/// validated as an explicit HTTPS origin. Endpoint validation occurs inside the
/// engine on the SAME discovery response subsequently used for both grants.
#[derive(Clone)]
pub(crate) struct StrictWorkspace(Url);

impl StrictWorkspace {
    fn new(raw: &str) -> Result<Self, AgentError> {
        let rest = raw.strip_prefix("https://").ok_or_else(invalid_workspace)?;
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.contains(['/', '@']) {
            return Err(invalid_workspace());
        }
        let url = Url::parse(raw).map_err(|_| invalid_workspace())?;
        if raw.trim() != raw
            || raw.chars().any(char::is_control)
            || raw.contains('\\')
            || url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(invalid_workspace());
        }
        Ok(Self(url))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str().trim_end_matches('/')
    }

    pub(crate) fn validate_endpoint(&self, raw: &str) -> Result<(), AgentError> {
        let authority = raw
            .strip_prefix("https://")
            .ok_or_else(invalid_endpoint)?
            .split('/')
            .next()
            .ok_or_else(invalid_endpoint)?;
        if authority.contains('@') {
            return Err(invalid_endpoint());
        }
        let endpoint = Url::parse(raw).map_err(|_| invalid_endpoint())?;
        if endpoint.origin() != self.0.origin()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || raw.trim() != raw
            || raw.chars().any(char::is_control)
            || raw.contains('\\')
        {
            return Err(invalid_endpoint());
        }
        Ok(())
    }
}

fn invalid_workspace() -> AgentError {
    AgentError::InvalidParams("Databricks workspace must be an explicit HTTPS origin".into())
}
fn invalid_endpoint() -> AgentError {
    AgentError::Llm("Databricks OAuth endpoint outside workspace policy".into())
}

#[cfg(test)]
#[path = "databricks_tests.rs"]
mod tests;
