//! The relay's feature-flag provider, chosen once at startup.
//!
//! `BUZZ_FEATURE_FLAGS_PROVIDER` selects exactly one evaluator for the life of
//! the process (see `crates/buzz-feature-flags/README.md`):
//!
//! | Value | Evaluator |
//! |-------|-----------|
//! | unset, `static` | every flag returns its declared default |
//! | `environment` | `BUZZ_FEATURE_FLAG_*` values, read once at startup |
//! | `launchdarkly` | LaunchDarkly, keyed by `BUZZ_LAUNCHDARKLY_SDK_KEY`, optionally through the Relay Proxy at `BUZZ_LAUNCHDARKLY_RELAY_PROXY_URL` |
//!
//! The variable is deliberately outside the `BUZZ_FEATURE_FLAG_` namespace the
//! environment evaluator reads, so it can never be mistaken for a flag.
//!
//! Local configuration errors (an unknown provider, a missing SDK key, a
//! malformed proxy URL) fail startup. LaunchDarkly itself being unreachable
//! does not: the relay starts on declared defaults and the SDK keeps
//! connecting in the background, because no safety invariant may depend on
//! remote flag availability. After LaunchDarkly first delivers data, a lost
//! connection leaves its last-known values in force until it reconnects.

use std::sync::Arc;
use std::time::Duration;

use buzz_feature_flags::launchdarkly::{LaunchDarklyEvaluator, LaunchDarklyRuntimeConfig};
use buzz_feature_flags::{EnvironmentEvaluator, FlagEvaluator, StaticEvaluator};
use tracing::{info, warn};

/// Environment variable naming the provider.
pub const PROVIDER_ENV: &str = "BUZZ_FEATURE_FLAGS_PROVIDER";
const SDK_KEY_ENV: &str = "BUZZ_LAUNCHDARKLY_SDK_KEY";
const RELAY_PROXY_ENV: &str = "BUZZ_LAUNCHDARKLY_RELAY_PROXY_URL";

/// How long startup waits for LaunchDarkly's first flag data before serving
/// on declared defaults.
const LAUNCHDARKLY_START_WAIT: Duration = Duration::from_secs(3);

/// How long shutdown waits for LaunchDarkly's final analytics flush. The flush
/// has no deadline of its own, and an unreachable events endpoint must not
/// spend the pod's termination grace.
const LAUNCHDARKLY_CLOSE_WAIT: Duration = Duration::from_secs(2);

/// The provider selected by [`PROVIDER_ENV`].
#[derive(Debug)]
enum Provider {
    Static,
    Environment,
    LaunchDarkly(LaunchDarklyRuntimeConfig),
}

/// Parse the provider from `get`, which reads one environment variable.
fn provider_from(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Provider> {
    let value = get(PROVIDER_ENV).unwrap_or_default();
    match value.trim() {
        "" | "static" => Ok(Provider::Static),
        "environment" => Ok(Provider::Environment),
        "launchdarkly" => {
            let sdk_key = get(SDK_KEY_ENV)
                .filter(|key| !key.trim().is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("{PROVIDER_ENV}=launchdarkly requires {SDK_KEY_ENV}")
                })?;
            let mut config = LaunchDarklyRuntimeConfig::new(sdk_key);
            if let Some(endpoint) = get(RELAY_PROXY_ENV).filter(|url| !url.trim().is_empty()) {
                config = config.with_relay_proxy_endpoint(endpoint);
            }
            Ok(Provider::LaunchDarkly(config))
        }
        other => anyhow::bail!(
            "{PROVIDER_ENV}={other:?} is not a feature flag provider; \
             use static, environment, or launchdarkly"
        ),
    }
}

/// The process's one flag evaluator, plus the concrete handle its provider
/// needs for diagnostics and shutdown.
pub struct FeatureFlags {
    evaluator: Arc<dyn FlagEvaluator>,
    provider: &'static str,
    environment: Option<Arc<EnvironmentEvaluator>>,
    launchdarkly: Option<Arc<LaunchDarklyEvaluator>>,
}

impl FeatureFlags {
    /// Build the provider [`PROVIDER_ENV`] selects. Fails only on local
    /// configuration errors; see the module docs.
    pub async fn from_env() -> anyhow::Result<Self> {
        let provider = provider_from(|name| std::env::var(name).ok())?;
        let flags = Self::build(provider, LAUNCHDARKLY_START_WAIT).await?;
        info!(provider = flags.provider, "feature flag provider selected");
        Ok(flags)
    }

    async fn build(provider: Provider, start_wait: Duration) -> anyhow::Result<Self> {
        Ok(match provider {
            Provider::Static => Self::from_evaluator("static", Arc::new(StaticEvaluator)),
            Provider::Environment => {
                let owner = Arc::new(EnvironmentEvaluator::from_process_environment());
                Self {
                    evaluator: owner.clone(),
                    provider: "environment",
                    environment: Some(owner),
                    launchdarkly: None,
                }
            }
            Provider::LaunchDarkly(config) => {
                let owner = Arc::new(
                    LaunchDarklyEvaluator::from_runtime_config(config)
                        .map_err(|e| anyhow::anyhow!("LaunchDarkly configuration: {e}"))?,
                );
                if let Err(error) = owner.start_with_default_executor_and_wait(start_wait).await {
                    warn!(
                        %error,
                        "LaunchDarkly has not delivered flags; serving declared defaults until it does"
                    );
                }
                Self {
                    evaluator: owner.clone(),
                    provider: "launchdarkly",
                    environment: None,
                    launchdarkly: Some(owner),
                }
            }
        })
    }

    /// Flags backed by `evaluator` alone (tests, embedding).
    pub fn from_evaluator(provider: &'static str, evaluator: Arc<dyn FlagEvaluator>) -> Self {
        Self {
            evaluator,
            provider,
            environment: None,
            launchdarkly: None,
        }
    }

    /// The evaluator every consumer shares.
    pub fn evaluator(&self) -> &dyn FlagEvaluator {
        self.evaluator.as_ref()
    }

    /// Report provider health after a round of evaluations: log malformed
    /// environment values once each, and publish whether the provider has
    /// initialized (`buzz_feature_flags_ready`). It does not track the
    /// connection afterwards: an initialized LaunchDarkly client keeps serving
    /// its last-known values through an outage.
    pub fn report(&self) {
        if let Some(environment) = &self.environment {
            for diagnostic in environment.take_diagnostics() {
                warn!(
                    ?diagnostic,
                    "malformed feature flag environment value; using its default"
                );
            }
        }
        let ready = self
            .launchdarkly
            .as_ref()
            .is_none_or(|launchdarkly| launchdarkly.initialized());
        metrics::gauge!("buzz_feature_flags_ready", "provider" => self.provider).set(if ready {
            1.0
        } else {
            0.0
        });
    }

    /// Stop the provider's background work, waiting at most
    /// [`LAUNCHDARKLY_CLOSE_WAIT`].
    pub async fn close(&self) {
        self.close_within(LAUNCHDARKLY_CLOSE_WAIT).await;
    }

    /// LaunchDarkly's close blocks while it flushes analytics, so it runs on
    /// its own detached thread: past `wait`, shutdown moves on and process
    /// exit ends the flush. (The blocking pool would not do: the runtime
    /// waits for its tasks when it drops.)
    async fn close_within(&self, wait: Duration) {
        let Some(launchdarkly) = self.launchdarkly.clone() else {
            return;
        };
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("launchdarkly-close".into())
            .spawn(move || {
                launchdarkly.close();
                let _ = closed_tx.send(());
            });
        if let Err(error) = spawned {
            warn!(%error, "LaunchDarkly shutdown failed");
            return;
        }
        match tokio::time::timeout(wait, closed_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => warn!("LaunchDarkly shutdown failed"),
            Err(_) => warn!(
                ?wait,
                "LaunchDarkly did not finish its final flush; shutting down without it"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(pairs: &[(&str, &str)]) -> anyhow::Result<Provider> {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        provider_from(|name| env.get(name).cloned())
    }

    #[test]
    fn provider_selection_is_one_explicit_choice() {
        assert!(matches!(parse(&[]), Ok(Provider::Static)));
        assert!(matches!(
            parse(&[(PROVIDER_ENV, "static")]),
            Ok(Provider::Static)
        ));
        assert!(matches!(
            parse(&[(PROVIDER_ENV, " environment ")]),
            Ok(Provider::Environment)
        ));
        // An SDK key alone never selects LaunchDarkly.
        assert!(matches!(
            parse(&[(SDK_KEY_ENV, "sdk-key")]),
            Ok(Provider::Static)
        ));

        let Ok(Provider::LaunchDarkly(config)) = parse(&[
            (PROVIDER_ENV, "launchdarkly"),
            (SDK_KEY_ENV, "sdk-key"),
            (RELAY_PROXY_ENV, "http://ld-proxy.example.org:8030"),
        ]) else {
            panic!("launchdarkly with a key selects LaunchDarkly");
        };
        assert_eq!(config.sdk_key, "sdk-key");
        assert_eq!(
            config.relay_proxy_endpoint.as_deref(),
            Some("http://ld-proxy.example.org:8030")
        );
        let Ok(Provider::LaunchDarkly(config)) = parse(&[
            (PROVIDER_ENV, "launchdarkly"),
            (SDK_KEY_ENV, "sdk-key"),
            (RELAY_PROXY_ENV, ""),
        ]) else {
            panic!("an empty proxy URL means no proxy");
        };
        assert_eq!(config.relay_proxy_endpoint, None);
    }

    #[test]
    fn provider_misconfiguration_fails_startup() {
        for (pairs, needle) in [
            (&[(PROVIDER_ENV, "launchdarkly")][..], SDK_KEY_ENV),
            (
                &[(PROVIDER_ENV, "launchdarkly"), (SDK_KEY_ENV, " ")][..],
                SDK_KEY_ENV,
            ),
            (
                &[(PROVIDER_ENV, "LaunchDarkly")][..],
                "not a feature flag provider",
            ),
            (
                &[(PROVIDER_ENV, "flagsmith")][..],
                "not a feature flag provider",
            ),
        ] {
            let error = parse(pairs)
                .expect_err("misconfiguration must fail")
                .to_string();
            assert!(error.contains(needle), "{pairs:?}: {error}");
        }
    }

    #[tokio::test]
    async fn an_unreachable_launchdarkly_starts_on_defaults() {
        // The relay installs its rustls provider before building flags; the
        // SDK's HTTP client needs one, and several are compiled in.
        let _already_installed = rustls::crypto::ring::default_provider().install_default();
        // Nothing listens on the discard port, so the SDK never initializes.
        let provider = parse(&[
            (PROVIDER_ENV, "launchdarkly"),
            (SDK_KEY_ENV, "sdk-key"),
            (RELAY_PROXY_ENV, "http://127.0.0.1:9"),
        ])
        .expect("valid configuration");
        let flags = FeatureFlags::build(provider, Duration::from_millis(50))
            .await
            .expect("an unreachable provider must not fail startup");
        let flag = buzz_feature_flags::IntegerFlag::new("buzz.relay.concurrency.auth", 0);
        assert_eq!(
            flags
                .evaluator()
                .evaluate_int(flag, &buzz_feature_flags::EvaluationContext::for_process()),
            0,
            "an uninitialized provider serves the declared default"
        );
        flags.close().await;
    }

    #[tokio::test]
    async fn close_is_bounded_when_launchdarkly_hangs() {
        let _already_installed = rustls::crypto::ring::default_provider().install_default();
        // A proxy that accepts connections and never answers: every SDK
        // request, the final analytics flush included, hangs.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                held.push(stream);
            }
        });
        let provider = parse(&[
            (PROVIDER_ENV, "launchdarkly"),
            (SDK_KEY_ENV, "sdk-key"),
            (RELAY_PROXY_ENV, &format!("http://{address}")),
        ])
        .expect("valid configuration");
        let flags = FeatureFlags::build(provider, Duration::from_millis(50))
            .await
            .expect("a hanging provider must not fail startup");
        // An evaluation queues an analytics event for the final flush.
        let flag = buzz_feature_flags::IntegerFlag::new("buzz.relay.concurrency.auth", 0);
        flags
            .evaluator()
            .evaluate_int(flag, &buzz_feature_flags::EvaluationContext::for_process());

        let started = std::time::Instant::now();
        flags.close_within(Duration::from_millis(200)).await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "close returns at its deadline, not when the flush gives up: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_malformed_proxy_url_fails_startup() {
        let provider = parse(&[
            (PROVIDER_ENV, "launchdarkly"),
            (SDK_KEY_ENV, "sdk-key"),
            (RELAY_PROXY_ENV, "ld-proxy.example.org"),
        ])
        .expect("parses");
        let Err(error) = FeatureFlags::build(provider, Duration::from_millis(50)).await else {
            panic!("a relative proxy URL must fail startup");
        };
        assert!(
            error.to_string().contains("LaunchDarkly configuration"),
            "{error}"
        );
    }
}
