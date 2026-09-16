use launchdarkly_server_sdk::{
    BuildError, Client, ConfigBuildError, ConfigBuilder, Context, ContextBuilder,
    MultiContextBuilder,
};
use std::time::Duration;
use thiserror::Error;

use crate::{BooleanFlag, BooleanFlagEvaluator, EvaluationContext};

const COMMUNITY_CONTEXT_KIND: &str = "community";
const PUBKEY_CONTEXT_KIND: &str = "pubkey";

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
        let context = match launchdarkly_context(context) {
            Ok(context) => context,
            Err(_) => return flag.default(),
        };
        self.client
            .bool_variation(&context, flag.key(), flag.default())
    }
}

fn launchdarkly_context(context: &EvaluationContext) -> Result<Context, String> {
    let mut community = ContextBuilder::new(context.community().to_string());
    community.kind(COMMUNITY_CONTEXT_KIND);
    let community = community.build()?;

    let Some(actor_pubkey) = context.actor_pubkey() else {
        return Ok(community);
    };

    let mut actor = ContextBuilder::new(actor_pubkey.to_hex());
    actor.kind(PUBKEY_CONTEXT_KIND);
    let actor = actor.build()?;

    MultiContextBuilder::of(vec![community, actor]).build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::{CommunityId, PublicKey};
    use launchdarkly_server_sdk::{FlagBuilder, FlagValue, Kind, TestData};

    fn community(id: &str) -> CommunityId {
        CommunityId::from_uuid(id.parse().expect("valid community UUID"))
    }

    fn actor() -> PublicKey {
        PublicKey::from_hex("5581946f95a03e6afb43027ec89b21507f040d35fd6f8594f168f4298f96f9cb")
            .expect("valid pubkey")
    }

    fn kind(name: &'static str) -> Kind {
        Kind::try_from(name).expect("valid LaunchDarkly context kind")
    }

    async fn started_evaluator(test_data: &TestData) -> LaunchDarklyEvaluator {
        let config = ConfigBuilder::new("sdk-key")
            .data_source(test_data)
            .build()
            .expect("launchdarkly test config");
        let client = Client::build(config).expect("launchdarkly test client");
        let evaluator = LaunchDarklyEvaluator::new(client);
        evaluator
            .start_with_default_executor_and_wait(Duration::from_secs(1))
            .await
            .expect("launchdarkly initialized");
        evaluator
    }

    #[test]
    fn runtime_config_debug_redacts_sdk_key() {
        let config = LaunchDarklyRuntimeConfig::new("super-secret-sdk-key")
            .with_relay_proxy_endpoint("https://relay.internal");
        let debug = format!("{config:?}");

        assert!(!debug.contains("super-secret-sdk-key"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("https://relay.internal"));
    }

    #[tokio::test]
    async fn community_only_context_targets_named_community_kind() {
        let community = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("community-only")
                .fallthrough_variation(false)
                .variation_for_key(kind("community"), community.to_string(), true),
        );
        let evaluator = started_evaluator(&test_data).await;

        assert!(evaluator.evaluate_bool(
            BooleanFlag::new("community-only", false),
            &EvaluationContext::for_community(community),
        ));
        evaluator.close();
    }

    #[tokio::test]
    async fn actor_context_targets_community_and_pubkey_kinds() {
        let community = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let actor = actor();
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("by-community")
                .fallthrough_variation(false)
                .variation_for_key(kind("community"), community.to_string(), true),
        );
        test_data.update(
            FlagBuilder::new("by-pubkey")
                .fallthrough_variation(false)
                .variation_for_key(kind("pubkey"), actor.to_hex(), true),
        );
        let evaluator = started_evaluator(&test_data).await;
        let context = EvaluationContext::for_actor(community, actor);

        assert!(evaluator.evaluate_bool(BooleanFlag::new("by-community", false), &context));
        assert!(evaluator.evaluate_bool(BooleanFlag::new("by-pubkey", false), &context));
        evaluator.close();
    }

    #[tokio::test]
    async fn same_actor_can_receive_different_variations_by_community() {
        let community_a = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let community_b = community("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let actor = actor();
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("community-rollout")
                .fallthrough_variation(false)
                .variation_for_key(kind("community"), community_a.to_string(), true),
        );
        let evaluator = started_evaluator(&test_data).await;
        let flag = BooleanFlag::new("community-rollout", false);

        assert!(evaluator.evaluate_bool(flag, &EvaluationContext::for_actor(community_a, actor),));
        assert!(!evaluator.evaluate_bool(flag, &EvaluationContext::for_actor(community_b, actor),));
        evaluator.close();
    }

    #[tokio::test]
    async fn missing_flag_falls_back_to_declared_default() {
        let test_data = TestData::new();
        let evaluator = started_evaluator(&test_data).await;
        let context =
            EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

        assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.feature.missing", true), &context,));
        evaluator.close();
    }

    #[tokio::test]
    async fn wrong_type_falls_back_to_declared_default() {
        let test_data = TestData::new();
        test_data.update(FlagBuilder::new("relay.feature.bool").variation_for_all(true));
        test_data.update(
            FlagBuilder::new("relay.feature.string")
                .value_for_all(FlagValue::Str("red".to_owned())),
        );
        let evaluator = started_evaluator(&test_data).await;
        let context = EvaluationContext::for_actor(
            community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            actor(),
        );

        assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.feature.bool", false), &context,));
        assert!(
            !evaluator.evaluate_bool(BooleanFlag::new("relay.feature.string", false), &context,)
        );
        evaluator.close();
    }
}
