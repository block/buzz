use std::sync::Arc;

use nostr::{NostrSigner, PublicKey};

use crate::{AuthContext, ClientConfig, ClientError, CommunityEndpoint};

/// Client session bound to one Buzz community and signer.
#[derive(Debug)]
pub struct BuzzClient {
    endpoint: CommunityEndpoint,
    pub(crate) signer: Arc<dyn NostrSigner>,
    public_key: PublicKey,
    pub(crate) auth: AuthContext,
    config: ClientConfig,
    pub(crate) http: reqwest::Client,
}

impl BuzzClient {
    /// Start constructing a client for one community with an asynchronous signer.
    pub fn builder(endpoint: CommunityEndpoint, signer: Arc<dyn NostrSigner>) -> BuzzClientBuilder {
        BuzzClientBuilder {
            endpoint,
            signer,
            auth: AuthContext::default(),
            config: ClientConfig::default(),
        }
    }

    /// Configured community endpoint.
    pub fn endpoint(&self) -> &CommunityEndpoint {
        &self.endpoint
    }

    /// Public key obtained and cached from the asynchronous signer.
    pub fn public_key(&self) -> PublicKey {
        self.public_key
    }

    /// Explicit session configuration.
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Signer-bound authentication context applied by this session.
    pub fn auth_context(&self) -> &AuthContext {
        &self.auth
    }

    /// Backend class of the configured asynchronous signer.
    pub fn signer_backend(&self) -> nostr::signer::SignerBackend<'_> {
        self.signer.backend()
    }
}

/// Asynchronous builder for a [`BuzzClient`].
#[derive(Debug)]
pub struct BuzzClientBuilder {
    endpoint: CommunityEndpoint,
    signer: Arc<dyn NostrSigner>,
    auth: AuthContext,
    config: ClientConfig,
}

impl BuzzClientBuilder {
    /// Set signer-bound authentication material.
    pub fn auth_context(mut self, auth: AuthContext) -> Self {
        self.auth = auth;
        self
    }

    /// Set explicit timeouts and bounded retry settings.
    pub fn config(mut self, config: ClientConfig) -> Self {
        self.config = config;
        self
    }

    /// Validate the signer, authentication context, and transport configuration.
    pub async fn build(self) -> Result<BuzzClient, ClientError> {
        let public_key = self
            .signer
            .get_public_key()
            .await
            .map_err(ClientError::Signer)?;
        if let Some(ambient) = self.auth.ambient() {
            buzz_sdk::nip_oa::verify_auth_tag(&ambient.json, &public_key)
                .map_err(|error| ClientError::Authentication(error.to_string()))?;
        }
        let http = reqwest::Client::builder()
            .timeout(self.config.request_timeout)
            .connect_timeout(self.config.connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(ClientError::HttpClient)?;

        Ok(BuzzClient {
            endpoint: self.endpoint,
            signer: self.signer,
            public_key,
            auth: self.auth,
            config: self.config,
            http,
        })
    }
}
