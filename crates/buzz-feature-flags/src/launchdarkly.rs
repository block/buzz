use http::Uri;
use launchdarkly_server_sdk::{
    BuildError, Client, ConfigBuildError, ConfigBuilder, Context, ContextBuilder,
    MultiContextBuilder,
};
use std::time::Duration;
use thiserror::Error;

use crate::{BooleanFlag, EvaluationContext, FlagEvaluator, IntegerFlag};

const COMMUNITY_CONTEXT_KIND: &str = "community";
const PUBKEY_CONTEXT_KIND: &str = "pubkey";
const JSON_SAFE_INTEGER_MAX: i64 = 9_007_199_254_740_991;

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
    /// Caller provided an invalid relay-proxy endpoint.
    #[error("launchdarkly relay proxy endpoint must be an absolute http(s) URI with authority")]
    InvalidRelayProxyEndpoint(InvalidRelayProxyEndpointReason),
    /// LaunchDarkly SDK config build failed.
    #[error("launchdarkly config build failed: {0}")]
    ConfigBuild(#[from] ConfigBuildError),
    /// LaunchDarkly SDK client build failed.
    #[error("launchdarkly client build failed: {0}")]
    ClientBuild(#[from] BuildError),
}

/// Why a relay-proxy endpoint failed pre-validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidRelayProxyEndpointReason {
    /// Endpoint could not be parsed as a URI.
    MalformedUri,
    /// Endpoint scheme is not `http` or `https`.
    InvalidScheme,
    /// Endpoint has no authority section (`host[:port]`).
    MissingAuthority,
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

/// Feature-flag evaluator backed by the LaunchDarkly Rust server SDK.
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
            let relay_proxy_endpoint = validate_relay_proxy_endpoint(&relay_proxy_endpoint)?;
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

impl FlagEvaluator for LaunchDarklyEvaluator {
    fn evaluate_bool(&self, flag: BooleanFlag, context: &EvaluationContext) -> bool {
        let context = match launchdarkly_context(context) {
            Ok(context) => context,
            Err(_) => return flag.default(),
        };
        self.client
            .bool_variation(&context, flag.key(), flag.default())
    }

    fn evaluate_int(&self, flag: IntegerFlag, context: &EvaluationContext) -> i64 {
        let context = match launchdarkly_context(context) {
            Ok(context) => context,
            Err(_) => return flag.default(),
        };

        let detail =
            self.client
                .float_variation_detail(&context, flag.key(), flag.default() as f64);

        let Some(value) = detail.value else {
            return flag.default();
        };

        strict_f64_to_i64(value).unwrap_or_else(|| flag.default())
    }
}

fn strict_f64_to_i64(value: f64) -> Option<i64> {
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }

    if value.abs() > JSON_SAFE_INTEGER_MAX as f64 {
        return None;
    }

    let integer = value as i64;
    (integer as f64 == value).then_some(integer)
}

fn validate_relay_proxy_endpoint(endpoint: &str) -> Result<String, LaunchDarklyInitError> {
    let parsed = endpoint.parse::<Uri>().map_err(|_| {
        LaunchDarklyInitError::InvalidRelayProxyEndpoint(
            InvalidRelayProxyEndpointReason::MalformedUri,
        )
    })?;

    if !matches!(parsed.scheme_str(), Some("http" | "https")) {
        return Err(LaunchDarklyInitError::InvalidRelayProxyEndpoint(
            InvalidRelayProxyEndpointReason::InvalidScheme,
        ));
    }

    if parsed.authority().is_some() {
        return Ok(endpoint.to_owned());
    }

    Err(LaunchDarklyInitError::InvalidRelayProxyEndpoint(
        InvalidRelayProxyEndpointReason::MissingAuthority,
    ))
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

    #[test]
    fn malformed_relay_proxy_endpoint_returns_typed_error_without_panic() {
        let result = std::panic::catch_unwind(|| {
            LaunchDarklyEvaluator::from_runtime_config(
                LaunchDarklyRuntimeConfig::new("sdk-key")
                    .with_relay_proxy_endpoint("not a valid endpoint"),
            )
        });

        assert!(
            result.is_ok(),
            "invalid endpoint should return Err, not panic"
        );
        assert!(matches!(
            result.expect("catch_unwind result"),
            Err(LaunchDarklyInitError::InvalidRelayProxyEndpoint(_))
        ));
    }

    #[test]
    fn relay_proxy_endpoint_rejects_non_http_scheme_and_missing_authority() {
        let invalid_scheme = LaunchDarklyEvaluator::from_runtime_config(
            LaunchDarklyRuntimeConfig::new("sdk-key")
                .with_relay_proxy_endpoint("ftp://relay.internal:8030"),
        );
        assert!(matches!(
            invalid_scheme,
            Err(LaunchDarklyInitError::InvalidRelayProxyEndpoint(
                InvalidRelayProxyEndpointReason::InvalidScheme
            ))
        ));

        for endpoint in [
            "https://",
            "http:///missing-authority",
            "https:///still-missing-authority",
        ] {
            let result = LaunchDarklyEvaluator::from_runtime_config(
                LaunchDarklyRuntimeConfig::new("sdk-key").with_relay_proxy_endpoint(endpoint),
            );
            assert!(matches!(
                result,
                Err(LaunchDarklyInitError::InvalidRelayProxyEndpoint(_))
            ));
        }
    }

    #[test]
    fn relay_proxy_endpoint_accepts_absolute_http_and_https_uris() {
        let valid_cases = [
            "http://relay.internal:8030",
            "https://relay.internal",
            "https://relay.internal:8443/path-prefix",
        ];

        for endpoint in valid_cases {
            let result = LaunchDarklyEvaluator::from_runtime_config(
                LaunchDarklyRuntimeConfig::new("sdk-key").with_relay_proxy_endpoint(endpoint),
            );

            assert!(
                result.is_ok(),
                "expected valid endpoint {endpoint} to be accepted"
            );
        }
    }

    #[test]
    fn relay_proxy_endpoint_error_does_not_leak_raw_endpoint_value() {
        let endpoint = "https://bad endpoint with spaces";
        let error = LaunchDarklyEvaluator::from_runtime_config(
            LaunchDarklyRuntimeConfig::new("sdk-key").with_relay_proxy_endpoint(endpoint),
        )
        .expect_err("invalid endpoint should fail");

        assert!(!format!("{error}").contains(endpoint));
        assert!(!format!("{error:?}").contains(endpoint));
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

    #[tokio::test]
    async fn community_only_context_targets_integer_variation() {
        let community = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("community-int")
                .variations([FlagValue::from(0_i64), FlagValue::from(13_i64)])
                .fallthrough_variation_index(0)
                .variation_index_for_key(kind("community"), community.to_string(), 1),
        );
        let evaluator = started_evaluator(&test_data).await;

        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("community-int", -1),
                &EvaluationContext::for_community(community),
            ),
            13
        );
        evaluator.close();
    }

    #[tokio::test]
    async fn actor_context_targets_integer_variation_by_community_and_pubkey() {
        let community = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let actor = actor();
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("by-community-int")
                .variations([FlagValue::from(0_i64), FlagValue::from(8_i64)])
                .fallthrough_variation_index(0)
                .variation_index_for_key(kind("community"), community.to_string(), 1),
        );
        test_data.update(
            FlagBuilder::new("by-pubkey-int")
                .variations([FlagValue::from(0_i64), FlagValue::from(21_i64)])
                .fallthrough_variation_index(0)
                .variation_index_for_key(kind("pubkey"), actor.to_hex(), 1),
        );
        let evaluator = started_evaluator(&test_data).await;
        let context = EvaluationContext::for_actor(community, actor);

        assert_eq!(
            evaluator.evaluate_int(IntegerFlag::new("by-community-int", -1), &context),
            8
        );
        assert_eq!(
            evaluator.evaluate_int(IntegerFlag::new("by-pubkey-int", -1), &context),
            21
        );
        evaluator.close();
    }

    #[tokio::test]
    async fn same_actor_can_receive_different_integer_variations_by_community() {
        let community_a = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let community_b = community("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let actor = actor();
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("community-int-rollout")
                .variations([FlagValue::from(-11_i64), FlagValue::from(99_i64)])
                .fallthrough_variation_index(0)
                .variation_index_for_key(kind("community"), community_a.to_string(), 1),
        );
        let evaluator = started_evaluator(&test_data).await;

        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("community-int-rollout", -1),
                &EvaluationContext::for_actor(community_a, actor),
            ),
            99
        );
        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("community-int-rollout", -1),
                &EvaluationContext::for_actor(community_b, actor),
            ),
            -11
        );
        evaluator.close();
    }

    #[tokio::test]
    async fn missing_integer_flag_falls_back_to_declared_default() {
        let test_data = TestData::new();
        let evaluator = started_evaluator(&test_data).await;
        let context =
            EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

        assert_eq!(
            evaluator.evaluate_int(IntegerFlag::new("relay.feature.missing-int", -7), &context),
            -7
        );
        evaluator.close();
    }

    #[tokio::test]
    async fn wrong_type_integer_falls_back_to_declared_default() {
        let test_data = TestData::new();
        test_data
            .update(FlagBuilder::new("relay.feature.int").value_for_all(FlagValue::from(17_i64)));
        test_data.update(
            FlagBuilder::new("relay.feature.int-bool").value_for_all(FlagValue::from(true)),
        );
        let evaluator = started_evaluator(&test_data).await;
        let context = EvaluationContext::for_actor(
            community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            actor(),
        );

        assert_eq!(
            evaluator.evaluate_int(IntegerFlag::new("relay.feature.int", 0), &context),
            17
        );
        assert_eq!(
            evaluator.evaluate_int(IntegerFlag::new("relay.feature.int-bool", -5), &context),
            -5
        );
        evaluator.close();
    }

    #[tokio::test]
    async fn fractional_integer_variation_falls_back_to_declared_default() {
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("relay.feature.int-fractional-positive")
                .value_for_all(FlagValue::from(17.75_f64)),
        );
        test_data.update(
            FlagBuilder::new("relay.feature.int-fractional-negative")
                .value_for_all(FlagValue::from(-17.75_f64)),
        );
        let evaluator = started_evaluator(&test_data).await;
        let context = EvaluationContext::for_actor(
            community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            actor(),
        );

        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("relay.feature.int-fractional-positive", 123),
                &context,
            ),
            123
        );
        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("relay.feature.int-fractional-negative", -456),
                &context,
            ),
            -456
        );
        evaluator.close();
    }

    #[tokio::test]
    async fn integer_variation_honors_json_safe_integer_boundaries() {
        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("relay.feature.int-safe-max")
                .value_for_all(FlagValue::from(9_007_199_254_740_991_f64)),
        );
        test_data.update(
            FlagBuilder::new("relay.feature.int-safe-min")
                .value_for_all(FlagValue::from(-9_007_199_254_740_991_f64)),
        );
        test_data.update(
            FlagBuilder::new("relay.feature.int-out-of-range-positive")
                .value_for_all(FlagValue::from(9_007_199_254_740_992_f64)),
        );
        test_data.update(
            FlagBuilder::new("relay.feature.int-out-of-range-negative")
                .value_for_all(FlagValue::from(-9_007_199_254_740_992_f64)),
        );
        let evaluator = started_evaluator(&test_data).await;
        let context = EvaluationContext::for_actor(
            community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            actor(),
        );

        assert_eq!(
            evaluator.evaluate_int(IntegerFlag::new("relay.feature.int-safe-max", 0), &context),
            9_007_199_254_740_991_i64
        );
        assert_eq!(
            evaluator.evaluate_int(IntegerFlag::new("relay.feature.int-safe-min", 0), &context),
            -9_007_199_254_740_991_i64
        );
        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("relay.feature.int-out-of-range-positive", 11),
                &context,
            ),
            11
        );
        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("relay.feature.int-out-of-range-negative", -12),
                &context,
            ),
            -12
        );
        evaluator.close();
    }

    #[tokio::test]
    async fn negative_integer_variation_is_supported() {
        let community = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let test_data = TestData::new();
        test_data.update(FlagBuilder::new("negative-int").value_for_all(FlagValue::from(-42_i64)));
        let evaluator = started_evaluator(&test_data).await;

        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("negative-int", 3),
                &EvaluationContext::for_community(community),
            ),
            -42
        );
        evaluator.close();
    }
}
