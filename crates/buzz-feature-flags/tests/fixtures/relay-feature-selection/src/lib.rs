#![allow(dead_code)]

use std::sync::Arc;

use buzz_feature_flags::{EnvironmentEvaluator, FlagEvaluator};
#[cfg(feature = "static-feature-flags")]
use buzz_feature_flags::StaticEvaluator;
#[cfg(feature = "launchdarkly-feature-flags")]
use buzz_feature_flags::launchdarkly::{
    LaunchDarklyEvaluator, LaunchDarklyInitError, LaunchDarklyRuntimeConfig,
};

#[cfg(not(any(
    feature = "static-feature-flags",
    feature = "environment-feature-flags",
    feature = "launchdarkly-feature-flags",
)))]
compile_error!(
    "select exactly one relay feature flag provider feature: static-feature-flags, environment-feature-flags, or launchdarkly-feature-flags"
);

#[cfg(any(
    all(feature = "static-feature-flags", feature = "environment-feature-flags"),
    all(feature = "static-feature-flags", feature = "launchdarkly-feature-flags"),
    all(feature = "environment-feature-flags", feature = "launchdarkly-feature-flags"),
))]
compile_error!(
    "select exactly one relay feature flag provider feature: static-feature-flags, environment-feature-flags, or launchdarkly-feature-flags"
);

#[cfg(any(feature = "static-feature-flags", feature = "environment-feature-flags"))]
#[derive(Clone, Debug, Default)]
pub struct FeatureFlagRuntimeConfig;

#[cfg(feature = "launchdarkly-feature-flags")]
#[derive(Clone, Debug)]
pub struct FeatureFlagRuntimeConfig {
    pub sdk_key: String,
    pub relay_proxy_endpoint: Option<String>,
}

pub struct FeatureFlagCompositionRoot {
    pub feature_flags: Arc<dyn FlagEvaluator>,
    pub environment_diagnostics: Option<Arc<EnvironmentEvaluator>>,
    #[cfg(feature = "launchdarkly-feature-flags")]
    pub launchdarkly_lifecycle: Option<Arc<LaunchDarklyEvaluator>>,
}

#[cfg(feature = "static-feature-flags")]
pub fn build_feature_flag_evaluator(_config: FeatureFlagRuntimeConfig) -> FeatureFlagCompositionRoot {
    FeatureFlagCompositionRoot {
        feature_flags: Arc::new(StaticEvaluator),
        environment_diagnostics: None,
    }
}

#[cfg(feature = "environment-feature-flags")]
pub fn build_feature_flag_evaluator(
    _config: FeatureFlagRuntimeConfig,
) -> FeatureFlagCompositionRoot {
    let owner = Arc::new(EnvironmentEvaluator::from_process_environment());
    let feature_flags: Arc<dyn FlagEvaluator> = owner.clone();
    FeatureFlagCompositionRoot {
        feature_flags,
        environment_diagnostics: Some(owner),
    }
}

#[cfg(feature = "launchdarkly-feature-flags")]
pub fn build_feature_flag_evaluator(
    config: FeatureFlagRuntimeConfig,
) -> Result<FeatureFlagCompositionRoot, LaunchDarklyInitError> {
    let mut runtime_config = LaunchDarklyRuntimeConfig::new(config.sdk_key);
    if let Some(relay_proxy_endpoint) = config.relay_proxy_endpoint {
        runtime_config = runtime_config.with_relay_proxy_endpoint(relay_proxy_endpoint);
    }

    let owner = Arc::new(LaunchDarklyEvaluator::from_runtime_config(runtime_config)?);
    let feature_flags: Arc<dyn FlagEvaluator> = owner.clone();
    Ok(FeatureFlagCompositionRoot {
        feature_flags,
        environment_diagnostics: None,
        launchdarkly_lifecycle: Some(owner),
    })
}

#[cfg(feature = "static-feature-flags")]
pub fn compile_smoke() {
    let _ = build_feature_flag_evaluator(FeatureFlagRuntimeConfig);
}

#[cfg(feature = "environment-feature-flags")]
pub fn compile_smoke() {
    let _ = build_feature_flag_evaluator(FeatureFlagRuntimeConfig);
}

#[cfg(feature = "launchdarkly-feature-flags")]
pub fn compile_smoke() {
    let _ = build_feature_flag_evaluator(FeatureFlagRuntimeConfig {
        sdk_key: "fixture-sdk-key".to_owned(),
        relay_proxy_endpoint: None,
    });
}
