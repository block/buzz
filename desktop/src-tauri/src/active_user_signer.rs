//! An owned signing capability captured before an active-user operation awaits.

use std::sync::Arc;

use nostr::{
    signer::SignerBackend, util::BoxedFuture, Event, EventBuilder, Keys, NostrSigner, PublicKey,
    SignerError, UnsignedEvent,
};

/// Owner operations not covered by rust-nostr's signing/encryption interface.
/// Implementations belong to the captured identity; they must not resolve the
/// current user or fall back to another backend when an operation fails.
pub(crate) trait AgentCapabilities: std::fmt::Debug + Send + Sync {
    fn public_key(&self) -> PublicKey;

    fn authorize_agent<'a>(
        &'a self,
        agent: &'a PublicKey,
        conditions: &'a str,
    ) -> BoxedFuture<'a, Result<String, String>>;

    /// Invalid records yield `Ok(None)`; operational failures yield `Err`.
    /// Validation includes the event signature, envelope, and keyed address.
    fn read_agent_memory<'a>(
        &'a self,
        event: &'a Event,
        agent: &'a PublicKey,
    ) -> BoxedFuture<'a, Result<Option<buzz_core_pkg::engram::Body>, String>>;
}

impl AgentCapabilities for Keys {
    fn public_key(&self) -> PublicKey {
        Keys::public_key(self)
    }

    fn authorize_agent<'a>(
        &'a self,
        agent: &'a PublicKey,
        conditions: &'a str,
    ) -> BoxedFuture<'a, Result<String, String>> {
        Box::pin(async move {
            buzz_sdk_pkg::nip_oa::compute_auth_tag(self, agent, conditions)
                .map_err(|e| format!("failed to compute NIP-OA auth tag: {e}"))
        })
    }

    fn read_agent_memory<'a>(
        &'a self,
        event: &'a Event,
        agent: &'a PublicKey,
    ) -> BoxedFuture<'a, Result<Option<buzz_core_pkg::engram::Body>, String>> {
        Box::pin(async move {
            // The original engram listing verified before validate_and_decrypt,
            // which validates the envelope/keyed address but not the signature.
            if event.verify().is_err() {
                return Ok(None);
            }
            Ok(buzz_core_pkg::engram::validate_and_decrypt(
                event,
                agent,
                &self.public_key(),
                self.secret_key(),
                agent,
            )
            .ok())
        })
    }
}

/// Backend-owned lifetime of a captured capability. It must never resolve a
/// replacement identity. Local capabilities omit this contract and do not expire.
pub(crate) trait CapabilityLifetime: std::fmt::Debug + Send + Sync {
    /// Distinguishes replacement sessions even when their public keys match.
    fn generation(&self) -> u64;
    /// Reject a capability that can no longer authorize work.
    fn check(&self) -> Result<(), String>;
    /// Resolve when this capability can no longer authorize work.
    fn canceled(&self) -> BoxedFuture<'_, ()>;
}

/// A signer's immutable identity and its asynchronous rust-nostr capability.
/// No secret-key accessor is exposed by this boundary.
#[derive(Clone, Debug)]
pub(crate) struct ActiveUserSigner {
    public_key: PublicKey,
    signer: Arc<dyn NostrSigner>,
    agent_capabilities: Option<Arc<dyn AgentCapabilities>>,
    validity: Option<Arc<dyn CapabilityLifetime>>,
    local_crypto: bool,
}

impl ActiveUserSigner {
    /// Capture a local identity, deriving the cached public key from those keys.
    pub(crate) fn local(keys: Keys) -> Self {
        let keys = Arc::new(keys);
        Self {
            public_key: AgentCapabilities::public_key(keys.as_ref()),
            signer: keys.clone(),
            agent_capabilities: Some(keys),
            validity: None,
            local_crypto: true,
        }
    }

    /// Resolve the cached identity from the signer itself, never caller input.
    pub(crate) async fn new(signer: Arc<dyn NostrSigner>) -> Result<Self, String> {
        let public_key = signer.get_public_key().await.map_err(|e| e.to_string())?;
        Ok(Self {
            public_key,
            signer,
            agent_capabilities: None,
            validity: None,
            local_crypto: false,
        })
    }

    /// Bind this captured capability to a backend-owned immutable lifetime.
    pub(crate) fn with_lifetime(mut self, lifetime: Arc<dyn CapabilityLifetime>) -> Self {
        self.validity = Some(lifetime);
        self
    }

    /// Attest to one agent with the exact NIP-OA conditions, using this owner's
    /// captured backend. The JSON string is the existing SDK caller format.
    pub(crate) async fn authorize_agent(
        &self,
        agent: &PublicKey,
        conditions: &str,
    ) -> Result<String, String> {
        self.run(async {
            self.agent_capabilities
                .as_ref()
                .ok_or("signer has no owner authorization")?
                .authorize_agent(agent, conditions)
                .await
        })
        .await
    }

    /// Validate and decrypt an agent memory with this captured owner. Invalid
    /// records are skippable; an unavailable crypto capability is an operation
    /// failure and must never be presented as an empty memory listing.
    pub(crate) async fn read_agent_memory(
        &self,
        event: &Event,
        agent: &PublicKey,
    ) -> Result<Option<buzz_core_pkg::engram::Body>, String> {
        self.run(async {
            self.agent_capabilities
                .as_ref()
                .ok_or("signer cannot validate keyed agent memory addresses")?
                .read_agent_memory(event, agent)
                .await
        })
        .await
    }

    #[cfg(test)]
    pub(crate) fn with_test_authorization(self, keys: Keys) -> Self {
        self.with_test_agent_capabilities(Arc::new(keys))
    }

    /// Attach purpose-specific capabilities only when they attest to this identity.
    pub(crate) fn with_agent_capabilities(
        mut self,
        capabilities: Arc<dyn AgentCapabilities>,
    ) -> Result<Self, String> {
        if self.public_key != capabilities.public_key() {
            return Err("agent capabilities do not match signer identity".into());
        }
        self.agent_capabilities = Some(capabilities);
        Ok(self)
    }

    #[cfg(test)]
    fn with_test_agent_capabilities(self, capabilities: Arc<dyn AgentCapabilities>) -> Self {
        self.with_agent_capabilities(capabilities)
            .expect("test capabilities must match signer identity")
    }

    /// Distinguish invalid local ciphertext from unavailable cryptography.
    /// Only the locally constructed Keys capability gives a definitive negative verdict.
    pub(crate) async fn decrypt_record(
        &self,
        author: &PublicKey,
        content: &str,
    ) -> Result<Option<zeroize::Zeroizing<String>>, String> {
        self.check_valid()?;
        match self.nip44_decrypt(author, content).await {
            Ok(text) => Ok(Some(zeroize::Zeroizing::new(text))),
            Err(_) if self.has_local_crypto() && self.check_valid().is_ok() => Ok(None),
            Err(_) => Err("archive decryption backend unavailable".into()),
        }
    }

    /// Only a local library decrypt error is definitive ciphertext invalidity.
    /// Remote operational errors must never be treated as processed records.
    pub(crate) fn has_local_crypto(&self) -> bool {
        self.local_crypto
    }

    /// Native generation supplements pubkey equality, including same-key reauthentication.
    pub(crate) fn generation(&self) -> Option<u64> {
        self.validity.as_ref().map(|v| v.generation())
    }

    /// Reject a held capability after logout, expiry or replacement.
    pub(crate) fn check_valid(&self) -> Result<(), String> {
        self.validity.as_ref().map_or(Ok(()), |v| v.check())
    }

    /// Wait for this captured capability to end; local capabilities do not expire.
    pub(crate) async fn canceled(&self) {
        if let Some(validity) = &self.validity {
            validity.canceled().await;
        } else {
            std::future::pending::<()>().await;
        }
    }

    /// Fence an existing foreground effect, including admission, transport and result.
    /// An already-transmitted request cannot be recalled from the relay.
    pub(crate) async fn run<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        self.check_valid()?;
        let result = if let Some(validity) = &self.validity {
            tokio::select! { biased; _ = validity.canceled() => Err("native authentication canceled or expired".into()), result = work => result }
        } else {
            work.await
        };
        self.check_valid()?;
        result
    }

    /// The identity captured with this signing capability.
    pub(crate) fn public_key(&self) -> PublicKey {
        self.public_key
    }

    /// Borrow the existing library capability, including NIP-04 and NIP-44.
    pub(crate) fn signer(&self) -> &dyn NostrSigner {
        self
    }

    /// Build with the captured identity and asynchronously sign without re-fetching it.
    pub(crate) async fn sign_event(&self, builder: EventBuilder) -> Result<Event, String> {
        self.run(async {
            self.signer
                .sign_event(builder.build(self.public_key))
                .await
                .map_err(|e| e.to_string())
        })
        .await
    }
}

impl NostrSigner for ActiveUserSigner {
    fn backend(&self) -> SignerBackend<'_> {
        self.signer.backend()
    }

    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async {
            self.check_valid()
                .map_err(|e| SignerError::backend(std::io::Error::other(e)))?;
            Ok(self.public_key)
        })
    }

    fn sign_event(&self, unsigned: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        if self.validity.is_none() {
            return self.signer.sign_event(unsigned);
        }
        Box::pin(async move {
            self.run(async {
                self.signer
                    .sign_event(unsigned)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| SignerError::backend(std::io::Error::other(e)))
        })
    }

    fn nip04_encrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        if self.validity.is_none() {
            return self.signer.nip04_encrypt(public_key, content);
        }
        Box::pin(async move {
            self.run(async {
                self.signer
                    .nip04_encrypt(public_key, content)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| SignerError::backend(std::io::Error::other(e)))
        })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        if self.validity.is_none() {
            return self.signer.nip04_decrypt(public_key, content);
        }
        Box::pin(async move {
            self.run(async {
                self.signer
                    .nip04_decrypt(public_key, content)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| SignerError::backend(std::io::Error::other(e)))
        })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        if self.validity.is_none() {
            return self.signer.nip44_encrypt(public_key, content);
        }
        Box::pin(async move {
            self.run(async {
                self.signer
                    .nip44_encrypt(public_key, content)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| SignerError::backend(std::io::Error::other(e)))
        })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        public_key: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        if self.validity.is_none() {
            return self.signer.nip44_decrypt(public_key, content);
        }
        Box::pin(async move {
            self.run(async {
                self.signer
                    .nip44_decrypt(public_key, content)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| SignerError::backend(std::io::Error::other(e)))
        })
    }
}

#[cfg(test)]
#[path = "active_user_signer_tests.rs"]
pub(crate) mod tests;
