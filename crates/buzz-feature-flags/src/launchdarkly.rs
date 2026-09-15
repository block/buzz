use launchdarkly_server_sdk::{
    BuildError, Client, ConfigBuildError, ConfigBuilder, ContextBuilder,
};
use std::time::Duration;
use thiserror::Error;

use crate::{BooleanFlag, BooleanFlagEvaluator, EvaluationContext};

/// Runtime inputs required to initialize a LaunchDarkly-backed evaluator.
#[derive(Clone)]
pub struct LaunchDarklyRuntimeConfig {
    /// SDK key used to authenticate with LaunchDarkly.
    pub sdk_key: String,
    /// Optional Relay Proxy endpoint used for all LaunchDarkly service traffic.
    pub relay_proxy_endpoint: Option<String>,
}

impl std::fmt::Debug for LaunchDarklyRuntimeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchDarklyRuntimeConfig")
            .field("sdk_key", &"[REDACTED]")
            .field("relay_proxy_endpoint", &self.relay_proxy_endpoint)
            .finish()
    }
}

impl LaunchDarklyRuntimeConfig {
    /// Create a runtime config from explicit caller-provided values.
    pub fn new(sdk_key: impl Into<String>) -> Self {
        Self {
            sdk_key: sdk_key.into(),
            relay_proxy_endpoint: None,
        }
    }

    /// Set the optional Relay Proxy endpoint.
    pub fn with_relay_proxy_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.relay_proxy_endpoint = Some(endpoint.into());
        self
    }
}

/// Errors building a LaunchDarkly evaluator from runtime inputs.
#[derive(Debug, Error)]
pub enum LaunchDarklyInitError {
    /// Caller provided an empty SDK key.
    #[error("launchdarkly sdk key must not be empty")]
    EmptySdkKey,
    /// Caller provided an empty relay-proxy endpoint.
    #[error("launchdarkly relay proxy endpoint must not be empty")]
    EmptyRelayProxyEndpoint,
    /// LaunchDarkly SDK config build failed.
    #[error("launchdarkly config build failed: {0}")]
    ConfigBuild(#[from] ConfigBuildError),
    /// LaunchDarkly SDK client build failed.
    #[error("launchdarkly client build failed: {0}")]
    ClientBuild(#[from] BuildError),
}

/// Errors starting a LaunchDarkly evaluator and waiting for initialization.
#[derive(Debug, Error)]
pub enum LaunchDarklyStartError {
    /// LaunchDarkly client did not initialize before timeout.
    #[error("launchdarkly initialization timed out after {0:?}")]
    InitializationTimeout(Duration),
    /// LaunchDarkly client reported failed initialization.
    #[error("launchdarkly initialization failed")]
    InitializationFailed,
}

/// Boolean evaluator backed by the LaunchDarkly Rust server SDK.
pub struct LaunchDarklyEvaluator {
    client: Client,
}

impl std::fmt::Debug for LaunchDarklyEvaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchDarklyEvaluator").finish()
    }
}

impl LaunchDarklyEvaluator {
    /// Wrap an already-constructed LaunchDarkly client.
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// Build a LaunchDarkly client from explicit runtime configuration.
    pub fn from_runtime_config(
        config: LaunchDarklyRuntimeConfig,
    ) -> Result<Self, LaunchDarklyInitError> {
        let sdk_key = config.sdk_key.trim();
        if sdk_key.is_empty() {
            return Err(LaunchDarklyInitError::EmptySdkKey);
        }

        let mut builder = ConfigBuilder::new(sdk_key);
        if let Some(relay_proxy_endpoint) = config.relay_proxy_endpoint {
            let relay_proxy_endpoint = relay_proxy_endpoint.trim().to_owned();
            if relay_proxy_endpoint.is_empty() {
                return Err(LaunchDarklyInitError::EmptyRelayProxyEndpoint);
            }
            let mut service_endpoints = launchdarkly_server_sdk::ServiceEndpointsBuilder::new();
            service_endpoints.relay_proxy(&relay_proxy_endpoint);
            builder = builder.service_endpoints(&service_endpoints);
        }

        let config = builder.build()?;
        let client = Client::build(config)?;
        Ok(Self::new(client))
    }

    /// Start background tasks and wait for SDK initialization.
    ///
    /// This method must run inside a Tokio runtime.
    pub async fn start_with_default_executor_and_wait(
        &self,
        timeout: Duration,
    ) -> Result<(), LaunchDarklyStartError> {
        self.client.start_with_default_executor();

        match self.client.wait_for_initialization(timeout).await {
            Some(true) => Ok(()),
            Some(false) => Err(LaunchDarklyStartError::InitializationFailed),
            None => Err(LaunchDarklyStartError::InitializationTimeout(timeout)),
        }
    }

    /// Gracefully stop background tasks and flush pending analytics events.
    pub fn close(&self) {
        self.client.close();
    }
}

impl BooleanFlagEvaluator for LaunchDarklyEvaluator {
    fn evaluate_bool(&self, flag: BooleanFlag, context: &EvaluationContext) -> bool {
        let context = match ContextBuilder::new(context.actor_pubkey().to_hex()).build() {
            Ok(context) => context,
            Err(_) => return flag.default(),
        };
        self.client
            .bool_variation(&context, flag.key(), flag.default())
    }
}
