//! Origin-sealed assertion transport evidence for NIP-FI authorization.
//!
//! This module deliberately stops at the transport boundary. It does not
//! validate JWT claims, prepare admission, consume replay identities, or make
//! lifecycle decisions. Those operations remain the responsibility of the
//! canonical verifier and final-admission authority.

use std::fmt;

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::ProofTransport;

/// The closed assertion transport profile that produced sealed evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertionTransportProfile {
    /// A request-bound `trusted-proxy-hmac-v1` provenance field.
    TrustedProxyHmacV1,
}

/// Opaque identity for one trusted-proxy nonce that final admission must claim.
///
/// The key is a domain-separated one-way digest of the decoded nonce. It is
/// safe to persist, unlike the assertion and raw nonce, and remains stable if
/// one proxy serves multiple authorities. Final admission adds the frozen
/// server-owned authorization domain and claim kind. The exclusive retain
/// bound is the proxy timestamp plus the configured maximum provenance age.
#[derive(Clone, PartialEq, Eq)]
pub struct TrustedProxyNonceClaim {
    claim_key: [u8; 32],
    retain_until: DateTime<Utc>,
}

impl TrustedProxyNonceClaim {
    pub(crate) const fn new(claim_key: [u8; 32], retain_until: DateTime<Utc>) -> Self {
        Self {
            claim_key,
            retain_until,
        }
    }

    /// Privacy-safe key that must be inserted atomically at final admission.
    pub const fn claim_key(&self) -> &[u8; 32] {
        &self.claim_key
    }

    /// Exclusive finite retention bound consumed by canonical admission.
    pub const fn retain_until(&self) -> DateTime<Utc> {
        self.retain_until
    }
}

impl fmt::Debug for TrustedProxyNonceClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TrustedProxyNonceClaim([REDACTED])")
    }
}

/// Move-only assertion bytes sealed to exact authenticated ingress facts.
///
/// This type intentionally does not implement [`Clone`]. Its debug output
/// never exposes the assertion, nonce, authority, path, or their digests. A
/// caller may borrow the confidential assertion only to pass it directly to
/// the frozen canonical assertion verifier.
pub struct SealedTransportEvidence {
    authorization_domain: CommunityId,
    assertion: Box<str>,
    assertion_digest: [u8; 32],
    request_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
    transport: ProofTransport,
    proxy_expires_at: DateTime<Utc>,
    nonce_claim: TrustedProxyNonceClaim,
    profile: AssertionTransportProfile,
}

impl SealedTransportEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_trusted_proxy(
        authorization_domain: CommunityId,
        assertion: Box<str>,
        assertion_digest: [u8; 32],
        method: &[u8],
        authority: &[u8],
        path_and_query: &[u8],
        body_digest: [u8; 32],
        transport: ProofTransport,
        proxy_expires_at: DateTime<Utc>,
        nonce_claim: TrustedProxyNonceClaim,
    ) -> Self {
        let request_fingerprint = framed_fingerprint(
            b"buzz:nip-fi:trusted-proxy-request:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                method,
                authority,
                path_and_query,
                &body_digest,
                &[proof_transport_code(transport)],
            ],
        );
        let transport_context_fingerprint = framed_fingerprint(
            b"buzz:nip-fi:assertion-transport:v1",
            &[
                b"trusted-proxy-hmac-v1",
                authorization_domain.as_uuid().as_bytes(),
                authority,
                &[proof_transport_code(transport)],
            ],
        );
        Self {
            authorization_domain,
            assertion,
            assertion_digest,
            request_fingerprint,
            transport_context_fingerprint,
            transport,
            proxy_expires_at,
            nonce_claim,
            profile: AssertionTransportProfile::TrustedProxyHmacV1,
        }
    }

    /// Server-resolved authorization domain sealed before replay lookup.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Borrow the exact compact-JWS bytes after transport verification.
    ///
    /// The returned value is confidential and must not be logged, serialized,
    /// or placed in public errors. It still requires canonical JWT validation.
    pub fn confidential_assertion(&self) -> &str {
        &self.assertion
    }

    /// SHA-256 of the exact compact-JWS bytes after the Bearer scheme.
    pub const fn assertion_digest(&self) -> &[u8; 32] {
        &self.assertion_digest
    }

    /// Stable digest of exact method, authority, path/query, and body digest.
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    /// Stable digest of the authenticated transport profile and authority.
    pub const fn transport_context_fingerprint(&self) -> &[u8; 32] {
        &self.transport_context_fingerprint
    }

    /// Server-selected proof transport sealed into the HMAC request binding.
    pub const fn transport(&self) -> ProofTransport {
        self.transport
    }

    /// Exclusive finite provenance expiry.
    pub const fn proxy_expires_at(&self) -> DateTime<Utc> {
        self.proxy_expires_at
    }

    /// Read-only nonce identity that final admission must claim atomically.
    pub const fn nonce_claim(&self) -> &TrustedProxyNonceClaim {
        &self.nonce_claim
    }

    /// Assertion transport profile selected by trusted server configuration.
    pub const fn profile(&self) -> AssertionTransportProfile {
        self.profile
    }
}

pub(crate) const fn proof_transport_code(transport: ProofTransport) -> u8 {
    match transport {
        ProofTransport::Nip42 => 1,
        ProofTransport::Nip98 => 2,
        ProofTransport::GitSmartHttpSession => 3,
        ProofTransport::Blossom => 4,
    }
}

impl fmt::Debug for SealedTransportEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SealedTransportEvidence([REDACTED])")
    }
}

fn framed_fingerprint(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}
