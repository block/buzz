//! The provider and model config in force for a session.
//!
//! buzz-agent builds both of these itself — `build_provider` resolves the
//! provider, and `model_config_from_user_config` the config — so it does not
//! need goose to hold them.
//!
//! It used to hand them to `Agent::update_provider`, which stores them on the
//! agent *and writes them to goose's sqlite session row*. That write was the
//! last thing putting a row in the database. Since the turn loop reads the
//! provider and model config back through this struct rather than through
//! goose, the round trip bought nothing.
//!
//! Two things still need goose's `Agent`: `list_tools` and
//! `dispatch_tool_call`. Neither touches the provider (`dispatch_tool_call`
//! contains no reference to it) or the session store.

use std::sync::Arc;

use goose::providers::base::Provider;
use goose_providers::model::ModelConfig;
use tokio::sync::RwLock;

/// Provider and model config for one session, swappable by `session/set_model`.
#[derive(Clone)]
pub struct SessionModel {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    provider: Arc<dyn Provider>,
    config: ModelConfig,
}

impl SessionModel {
    pub fn new(provider: Arc<dyn Provider>, config: ModelConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner { provider, config })),
        }
    }

    /// The provider to stream from.
    pub async fn provider(&self) -> Arc<dyn Provider> {
        Arc::clone(&self.inner.read().await.provider)
    }

    /// The model config, which also carries the context limit compaction needs.
    pub async fn config(&self) -> ModelConfig {
        self.inner.read().await.config.clone()
    }

    /// Swap both, as `session/set_model` does.
    ///
    /// Taken together under one write so a turn can never observe a new
    /// provider with the previous model's config.
    pub async fn set(&self, provider: Arc<dyn Provider>, config: ModelConfig) {
        let mut inner = self.inner.write().await;
        inner.provider = provider;
        inner.config = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goose_provider_types::conversation::message::Message;
    use goose_provider_types::errors::ProviderError;

    /// Minimal provider that reports its own name, so a swap is observable.
    struct NamedProvider(&'static str);

    #[async_trait::async_trait]
    impl Provider for NamedProvider {
        fn get_name(&self) -> &str {
            self.0
        }

        async fn stream(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[rmcp::model::Tool],
        ) -> Result<goose::providers::base::MessageStream, ProviderError> {
            unreachable!("tests never call the model")
        }
    }

    fn model(name: &'static str) -> (Arc<dyn Provider>, ModelConfig) {
        (Arc::new(NamedProvider(name)), ModelConfig::new(name))
    }

    #[tokio::test]
    async fn the_configured_provider_and_config_are_what_come_back() {
        let (provider, config) = model("first");
        let session = SessionModel::new(provider, config);

        assert_eq!(session.provider().await.get_name(), "first");
        assert_eq!(session.config().await.model_name, "first");
    }

    #[tokio::test]
    async fn set_model_swaps_provider_and_config_together() {
        // The pair must move as one: a turn seeing a new provider with the old
        // config would stream against the wrong context limit.
        let (provider, config) = model("first");
        let session = SessionModel::new(provider, config);

        let (next_provider, next_config) = model("second");
        session.set(next_provider, next_config).await;

        assert_eq!(session.provider().await.get_name(), "second");
        assert_eq!(session.config().await.model_name, "second");
    }

    #[tokio::test]
    async fn clones_share_one_model() {
        // `SessionModel` is cloned into the turn; a `session/set_model` applied
        // through one handle has to be visible through the other.
        let (provider, config) = model("first");
        let session = SessionModel::new(provider, config);
        let clone = session.clone();

        let (next_provider, next_config) = model("second");
        session.set(next_provider, next_config).await;

        assert_eq!(clone.provider().await.get_name(), "second");
    }
}
