//! Relay-authenticated authority carried only across ephemeral Redis fan-out.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buzz_core::CommunityId;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::executor::{revalidate_ephemeral_claim, EphemeralAuthorityClaim};
use super::transport::ProtectedAuthorization;
use crate::connection::QueuedOutboundReleaseFence;
use crate::state::AppState;

const DOMAIN_SEPARATOR: &[u8] = b"buzz-ephemeral-redis-authority-v1";
type HmacSha256 = Hmac<Sha256>;
const PRESENCE_PREFIX: &str = "pa1:";

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct ProtectedPresenceValue {
    pub(crate) status: String,
    pub(crate) context_id: [u8; 32],
    pub(crate) authority: String,
}

impl fmt::Debug for ProtectedPresenceValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedPresenceValue")
            .field("status", &"[redacted]")
            .field("context_id", &"[redacted]")
            .field("authority", &"[redacted]")
            .finish()
    }
}

pub(crate) fn encode_presence(
    status: String,
    context_id: [u8; 32],
    authority: String,
) -> Result<String, EphemeralAuthorityError> {
    let value = serde_json::to_vec(&ProtectedPresenceValue {
        status,
        context_id,
        authority,
    })?;
    Ok(format!(
        "{PRESENCE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(value)
    ))
}

pub(crate) fn decode_presence(
    value: &str,
) -> Result<Option<ProtectedPresenceValue>, EphemeralAuthorityError> {
    let Some(value) = value.strip_prefix(PRESENCE_PREFIX) else {
        return Ok(None);
    };
    let value = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| EphemeralAuthorityError::InvalidEnvelope)?;
    Ok(Some(serde_json::from_slice(&value)?))
}

/// Seal one provider-neutral sender authority for trusted relay Redis fan-out.
pub(crate) fn seal(
    state: &AppState,
    authority: &ProtectedAuthorization,
    event: &nostr::Event,
) -> Result<String, EphemeralAuthorityError> {
    seal_context(state, authority, event.id.to_bytes())
}

/// Seal enforcing authority for a non-event ephemeral effect boundary.
pub(crate) fn seal_context(
    state: &AppState,
    authority: &ProtectedAuthorization,
    context_id: [u8; 32],
) -> Result<String, EphemeralAuthorityError> {
    let claim = authority
        .seal_ephemeral_delivery(context_id)?
        .ok_or(EphemeralAuthorityError::AuthorityRequired)?;
    seal_claim(&signing_key(state), &claim)
}

fn seal_claim(
    signing_key: &[u8; 32],
    claim: &EphemeralAuthorityClaim,
) -> Result<String, EphemeralAuthorityError> {
    let payload = serde_json::to_vec(claim)?;
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(signing_key)
        .map_err(|_| EphemeralAuthorityError::InvalidSignature)?;
    mac.update(DOMAIN_SEPARATOR);
    mac.update(&(payload.len() as u64).to_be_bytes());
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

/// Verify and preflight a remote sender authority before any local fan-out.
pub(crate) async fn verify(
    state: &AppState,
    community_id: CommunityId,
    event: &nostr::Event,
    token: &str,
) -> Result<Arc<dyn QueuedOutboundReleaseFence>, EphemeralAuthorityError> {
    let claim = verify_claim(&signing_key(state), token)?;
    if claim.community_id != *community_id.as_uuid()
        || claim.event_id != event.id.to_bytes()
        || claim.actor_pubkey != event.pubkey.to_bytes()
        || claim.capability != "community_write"
    {
        return Err(EphemeralAuthorityError::ContextMismatch);
    }
    let authority = Arc::new(RemoteEphemeralAuthority {
        db: state.db.clone(),
        claim,
    });
    if !authority.release().await {
        return Err(EphemeralAuthorityError::ExpiredOrInvalidated);
    }
    Ok(authority)
}

/// Database/signing material retained by an internal cross-node effect owner.
#[derive(Clone)]
pub(crate) enum AuthorityTokenVerifier {
    Database {
        db: buzz_db::Db,
        signing_key: [u8; 32],
    },
    #[cfg(test)]
    TestAllow,
    #[cfg(test)]
    TestDeny,
    #[cfg(test)]
    TestConditional(Arc<std::sync::atomic::AtomicBool>),
}

/// Authority retained by an ephemeral effect owner through cleanup. The
/// absolute lease deadline is available to synchronous realtime boundaries;
/// invalidation and binding state remain asynchronously revalidated.
#[derive(Clone)]
pub(crate) struct RetainedEphemeralAuthority {
    fence: Arc<dyn QueuedOutboundReleaseFence>,
    expires_at: u64,
}

impl RetainedEphemeralAuthority {
    pub(crate) fn expires_at(&self) -> u64 {
        self.expires_at
    }

    pub(crate) async fn release(&self) -> bool {
        self.is_time_valid() && self.fence.release().await
    }

    pub(crate) fn is_time_valid(&self) -> bool {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .is_ok_and(|duration| duration.as_secs() < self.expires_at)
    }
}

impl AuthorityTokenVerifier {
    /// Build from the shared relay signing secret without retaining it.
    pub(crate) fn new(db: buzz_db::Db, relay_secret: &[u8]) -> Self {
        Self::Database {
            db,
            signing_key: signing_key_from_secret(relay_secret),
        }
    }

    #[cfg(test)]
    pub(crate) fn allow_for_test() -> Self {
        Self::TestAllow
    }

    #[cfg(test)]
    pub(crate) fn deny_for_test() -> Self {
        Self::TestDeny
    }

    #[cfg(test)]
    pub(crate) fn conditional_for_test(gate: Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self::TestConditional(gate)
    }

    /// Verify exact context and current database authority at the effect owner.
    pub(crate) async fn verify_context(
        &self,
        community_id: CommunityId,
        context_id: [u8; 32],
        token: &str,
    ) -> Result<RetainedEphemeralAuthority, EphemeralAuthorityError> {
        match self {
            Self::Database { db, signing_key } => {
                let claim = verify_claim(signing_key, token)?;
                if claim.community_id != *community_id.as_uuid()
                    || claim.event_id != context_id
                    || claim.capability != "audio_join"
                {
                    return Err(EphemeralAuthorityError::ContextMismatch);
                }
                let expires_at = claim.expires_at;
                let authority = RetainedEphemeralAuthority {
                    fence: Arc::new(RemoteEphemeralAuthority {
                        db: db.clone(),
                        claim,
                    }),
                    expires_at,
                };
                if !authority.release().await {
                    return Err(EphemeralAuthorityError::ExpiredOrInvalidated);
                }
                Ok(authority)
            }
            #[cfg(test)]
            Self::TestAllow => Ok(RetainedEphemeralAuthority {
                fence: Arc::new(TestConditionalAuthority {
                    gate: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                }),
                expires_at: test_authority_expiry(),
            }),
            #[cfg(test)]
            Self::TestDeny => Err(EphemeralAuthorityError::ExpiredOrInvalidated),
            #[cfg(test)]
            Self::TestConditional(gate) => {
                if !gate.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(EphemeralAuthorityError::ExpiredOrInvalidated);
                }
                Ok(RetainedEphemeralAuthority {
                    fence: Arc::new(TestConditionalAuthority {
                        gate: Arc::clone(gate),
                    }),
                    expires_at: test_authority_expiry(),
                })
            }
        }
    }

    /// Verify exact context, actor, and current database authority.
    pub(crate) async fn verify_actor_context(
        &self,
        community_id: CommunityId,
        context_id: [u8; 32],
        actor_pubkey: [u8; 32],
        token: &str,
    ) -> Result<(), EphemeralAuthorityError> {
        match self {
            Self::Database { db, signing_key } => {
                let claim = verify_claim(signing_key, token)?;
                if claim.community_id != *community_id.as_uuid()
                    || claim.event_id != context_id
                    || claim.actor_pubkey != actor_pubkey
                    || claim.capability != "community_write"
                {
                    return Err(EphemeralAuthorityError::ContextMismatch);
                }
                revalidate_ephemeral_claim(db, &claim)
                    .await
                    .map_err(|_| EphemeralAuthorityError::ExpiredOrInvalidated)
            }
            #[cfg(test)]
            Self::TestAllow => Ok(()),
            #[cfg(test)]
            Self::TestDeny => Err(EphemeralAuthorityError::ExpiredOrInvalidated),
            #[cfg(test)]
            Self::TestConditional(gate) => gate
                .load(std::sync::atomic::Ordering::SeqCst)
                .then_some(())
                .ok_or(EphemeralAuthorityError::ExpiredOrInvalidated),
        }
    }
}

#[cfg(test)]
fn test_authority_expiry() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("test wall clock after Unix epoch")
        .as_secs()
        + 3_600
}

#[cfg(test)]
struct TestConditionalAuthority {
    gate: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
#[async_trait]
impl QueuedOutboundReleaseFence for TestConditionalAuthority {
    async fn release(&self) -> bool {
        self.gate.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn verify_claim(
    signing_key: &[u8; 32],
    token: &str,
) -> Result<EphemeralAuthorityClaim, EphemeralAuthorityError> {
    let (payload, signature) = token
        .split_once('.')
        .ok_or(EphemeralAuthorityError::InvalidEnvelope)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| EphemeralAuthorityError::InvalidEnvelope)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| EphemeralAuthorityError::InvalidEnvelope)?;
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(signing_key)
        .map_err(|_| EphemeralAuthorityError::InvalidSignature)?;
    mac.update(DOMAIN_SEPARATOR);
    mac.update(&(payload.len() as u64).to_be_bytes());
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| EphemeralAuthorityError::InvalidSignature)?;
    Ok(serde_json::from_slice(&payload)?)
}

fn signing_key(state: &AppState) -> [u8; 32] {
    signing_key_from_secret(state.relay_keypair.secret_key().as_secret_bytes())
}

fn signing_key_from_secret(secret: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(DOMAIN_SEPARATOR);
    digest.update(secret);
    digest.finalize().into()
}

struct RemoteEphemeralAuthority {
    db: buzz_db::Db,
    claim: EphemeralAuthorityClaim,
}

#[async_trait]
impl QueuedOutboundReleaseFence for RemoteEphemeralAuthority {
    async fn release(&self) -> bool {
        revalidate_ephemeral_claim(&self.db, &self.claim)
            .await
            .is_ok()
    }
}

#[derive(Debug, Error)]
pub(crate) enum EphemeralAuthorityError {
    #[error("ephemeral sender authority is required")]
    AuthorityRequired,
    #[error("ephemeral sender authority envelope is invalid")]
    InvalidEnvelope,
    #[error("ephemeral sender authority signature is invalid")]
    InvalidSignature,
    #[error("ephemeral sender authority context does not match")]
    ContextMismatch,
    #[error("ephemeral sender authority is expired or invalidated")]
    ExpiredOrInvalidated,
    #[error("ephemeral sender authority could not be sealed")]
    Transport(#[from] super::transport::ProtectedTransportError),
    #[error("ephemeral sender authority serialization failed")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_presence_round_trips_exact_authority_context() {
        let context_id = [0x5a; 32];
        let encoded = encode_presence("online".into(), context_id, "sealed-authority".into())
            .expect("encode presence");
        let decoded = decode_presence(&encoded)
            .expect("decode presence")
            .expect("protected value");
        assert_eq!(decoded.status, "online");
        assert_eq!(decoded.context_id, context_id);
        assert_eq!(decoded.authority, "sealed-authority");
        let debug = format!("{decoded:?}");
        assert!(!debug.contains("online"));
        assert!(!debug.contains("sealed-authority"));
        assert!(!debug.contains("5a5a"));
    }

    #[test]
    fn legacy_presence_is_distinct_from_protected_presence() {
        assert!(decode_presence("online")
            .expect("legacy presence is valid")
            .is_none());
    }

    #[test]
    fn malformed_protected_presence_fails_closed() {
        assert!(matches!(
            decode_presence("pa1:not-base64***"),
            Err(EphemeralAuthorityError::InvalidEnvelope)
        ));
    }
}
