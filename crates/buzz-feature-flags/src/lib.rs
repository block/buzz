#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Typed boolean feature flags for Buzz server components.

use buzz_core::PublicKey;

#[cfg(feature = "launchdarkly")]
/// LaunchDarkly-backed evaluator adapter.
pub mod launchdarkly;

/// Typed boolean feature definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BooleanFlag {
    key: &'static str,
    default: bool,
}

impl BooleanFlag {
    /// Construct a typed boolean flag from its stable provider key and declared default.
    pub const fn new(key: &'static str, default: bool) -> Self {
        Self { key, default }
    }

    /// Provider key for this flag.
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// Declared default used when an evaluator is unavailable or cannot return a value.
    pub const fn default(self) -> bool {
        self.default
    }
}

/// Minimal evaluation context with one stable targeting identifier.
///
/// Buzz identities are Nostr keys. A pubkey is stable across relay sessions,
/// non-secret, and already first-class in existing server types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationContext {
    actor_pubkey: PublicKey,
}

impl EvaluationContext {
    /// Construct a context for evaluations that target one actor identity.
    pub fn for_pubkey(actor_pubkey: PublicKey) -> Self {
        Self { actor_pubkey }
    }

    /// Stable actor key for provider targeting.
    pub fn actor_pubkey(&self) -> &PublicKey {
        &self.actor_pubkey
    }
}

/// Evaluates typed boolean feature flags.
pub trait BooleanFlagEvaluator: Send + Sync {
    /// Resolve a boolean flag for one actor context.
    fn evaluate_bool(&self, flag: BooleanFlag, context: &EvaluationContext) -> bool;
}

/// Static evaluator that should return each flag's declared default.
#[derive(Debug, Default, Clone, Copy)]
pub struct StaticEvaluator;

impl BooleanFlagEvaluator for StaticEvaluator {
    fn evaluate_bool(&self, flag: BooleanFlag, _context: &EvaluationContext) -> bool {
        flag.default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_evaluator_returns_declared_default() {
        let context = EvaluationContext::for_pubkey(
            PublicKey::from_hex("c4f0623bdc8c4f7ecab9f7457f501f3e8f4efcf8f8f6ef6f4d76f42f5bb6f2cb")
                .expect("valid pubkey"),
        );
        let flag = BooleanFlag::new("relay.mesh_demo_echo", true);

        let got = StaticEvaluator.evaluate_bool(flag, &context);
        assert!(got);
    }

    #[cfg(feature = "launchdarkly")]
    #[tokio::test]
    async fn launchdarkly_missing_flag_falls_back_to_declared_default() {
        use launchdarkly_server_sdk::{Client, ConfigBuilder, TestData};
        use std::time::Duration;

        let test_data = TestData::new();

        let config = ConfigBuilder::new("sdk-key")
            .data_source(&test_data)
            .build()
            .expect("launchdarkly test config");
        let client = Client::build(config).expect("launchdarkly test client");

        let evaluator = crate::launchdarkly::LaunchDarklyEvaluator::new(client);
        evaluator
            .start_with_default_executor_and_wait(Duration::from_secs(1))
            .await
            .expect("launchdarkly initialized");
        let context = EvaluationContext::for_pubkey(
            PublicKey::from_hex("eb1539f3815379cbf4fdbb23609f730d9fce4fd2a7fbc6a93dbf39f8e9f4704d")
                .expect("valid pubkey"),
        );
        let flag = BooleanFlag::new("relay.feature.missing", true);

        let got = evaluator.evaluate_bool(flag, &context);
        assert!(got);
        evaluator.close();
    }

    #[cfg(feature = "launchdarkly")]
    #[tokio::test]
    async fn launchdarkly_type_error_falls_back_to_declared_default() {
        use launchdarkly_server_sdk::{Client, ConfigBuilder, FlagBuilder, FlagValue, TestData};
        use std::time::Duration;

        let test_data = TestData::new();
        test_data.update(
            FlagBuilder::new("relay.feature.bool")
                .variations(vec![FlagValue::Bool(true)])
                .fallthrough_variation_index(0),
        );
        test_data.update(
            FlagBuilder::new("relay.feature.string")
                .variations(vec![FlagValue::Str("red".to_owned())])
                .fallthrough_variation_index(0),
        );

        let config = ConfigBuilder::new("sdk-key")
            .data_source(&test_data)
            .build()
            .expect("launchdarkly test config");
        let client = Client::build(config).expect("launchdarkly test client");

        let evaluator = crate::launchdarkly::LaunchDarklyEvaluator::new(client);
        evaluator
            .start_with_default_executor_and_wait(Duration::from_secs(1))
            .await
            .expect("launchdarkly initialized");
        let context = EvaluationContext::for_pubkey(
            PublicKey::from_hex("5581946f95a03e6afb43027ec89b21507f040d35fd6f8594f168f4298f96f9cb")
                .expect("valid pubkey"),
        );

        let bool_flag = BooleanFlag::new("relay.feature.bool", false);
        let bool_got = evaluator.evaluate_bool(bool_flag, &context);
        assert!(bool_got);

        let flag = BooleanFlag::new("relay.feature.string", false);

        let got = evaluator.evaluate_bool(flag, &context);
        assert!(!got);
        evaluator.close();
    }
}
