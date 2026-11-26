//! The identity that signs the kind-9 messages the gateway sends.
//!
//! The relay's `POST /events` accepts an event only when its author is the
//! NIP-98 signer of the request, and a kind-9 message only when that author
//! can write to the channel. One signer therefore signs both the message and
//! the request that submits it, and may pick a different identity per
//! community, integration or channel.
use async_trait::async_trait;
use nostr::{Keys, NostrSigner};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

/// Where a message is going.
#[derive(Debug, Clone, Copy)]
pub struct SigningTarget<'a> {
    pub community_host: &'a str,
    pub integration_id: Uuid,
    pub subscription_id: Uuid,
    pub channel_id: Uuid,
}

/// Why no signer is available for a target.
#[derive(Debug, Error)]
pub enum SignerError {
    /// Transient: the message is retried with backoff.
    #[error("signer unavailable: {0}")]
    Unavailable(String),
    /// No identity can ever sign for this target: the message fails.
    #[error("no signer for this target: {0}")]
    Unconfigured(String),
}

/// Resolves the identity that authors a message and signs its submission.
#[async_trait]
pub trait MessageSigner: Send + Sync {
    /// The signer for the kind-9 event bound for `target`, also used for the
    /// NIP-98 authorization of the request that submits it.
    async fn signer_for(
        &self,
        target: &SigningTarget<'_>,
    ) -> Result<Arc<dyn NostrSigner>, SignerError>;
}

/// Placeholder [`MessageSigner`] that signs every message with the gateway's
/// own key.
///
/// TODO: replace with a per-community identity. The relay accepts a kind-9
/// message only from a member with write access to the channel, and the
/// gateway key is a member of no community, so until a real identity is wired
/// in the relay rejects these messages unless an operator adds the gateway
/// pubkey to the community and channel by hand.
pub struct GatewayKeySigner {
    signer: Arc<dyn NostrSigner>,
}

impl GatewayKeySigner {
    /// Sign every message with `keys`.
    pub fn new(keys: Keys) -> Self {
        Self {
            signer: Arc::new(keys),
        }
    }
}

#[async_trait]
impl MessageSigner for GatewayKeySigner {
    async fn signer_for(
        &self,
        _target: &SigningTarget<'_>,
    ) -> Result<Arc<dyn NostrSigner>, SignerError> {
        Ok(Arc::clone(&self.signer))
    }
}
