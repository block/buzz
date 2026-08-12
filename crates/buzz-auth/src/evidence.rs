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
    /// A request-bound `trusted-proxy-hmac-v2` field with authenticated peer.
    TrustedProxyHmacV2,
}

/// Opaque, authenticated end-client peer identity for bounded admission.
///
/// The key is a domain-separated keyed digest of the canonical peer address.
/// It may be used as a status-admission coordinate, but cannot reveal the raw
/// address or be constructed from an unauthenticated forwarding header.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedClientPeer {
    admission_key: [u8; 32],
}

impl AuthenticatedClientPeer {
    pub(crate) const fn new(admission_key: [u8; 32]) -> Self {
        Self { admission_key }
    }

    /// Construct deterministic opaque peer evidence for dev/test harnesses.
    ///
    /// This bypasses transport verification and is unavailable unless a test
    /// build or the explicitly development-only `dev` feature is selected.
    #[cfg(any(test, feature = "dev"))]
    pub const fn for_test(admission_key: [u8; 32]) -> Self {
        Self { admission_key }
    }

    /// Privacy-safe key for an admission counter.
    pub const fn admission_key(&self) -> &[u8; 32] {
        &self.admission_key
    }
}

impl fmt::Debug for AuthenticatedClientPeer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedClientPeer([REDACTED])")
    }
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
    authenticated_client_peer: Option<AuthenticatedClientPeer>,
}

impl SealedTransportEvidence {
    /// Construct origin-shaped opaque evidence for cross-crate dev tests.
    ///
    /// The caller supplies only an already opaque peer key; raw addresses are
    /// deliberately not accepted by this test seam.
    #[cfg(any(test, feature = "dev"))]
    #[allow(clippy::too_many_arguments)]
    pub fn for_test(
        authorization_domain: CommunityId,
        assertion: impl Into<Box<str>>,
        method: &[u8],
        authority: &[u8],
        path_and_query: &[u8],
        body_digest: [u8; 32],
        transport: ProofTransport,
        proxy_expires_at: DateTime<Utc>,
        authenticated_client_peer: AuthenticatedClientPeer,
    ) -> Self {
        Self::for_test_with_nonce(
            authorization_domain,
            assertion,
            method,
            authority,
            path_and_query,
            body_digest,
            transport,
            proxy_expires_at,
            authenticated_client_peer,
            [0x91; 32],
        )
    }

    /// Construct test evidence with an explicit opaque replay identity.
    ///
    /// This permits independent connections in one domain to exercise the
    /// production nonce ledger without deleting or bypassing prior claims.
    #[cfg(any(test, feature = "dev"))]
    #[allow(clippy::too_many_arguments)]
    pub fn for_test_with_nonce(
        authorization_domain: CommunityId,
        assertion: impl Into<Box<str>>,
        method: &[u8],
        authority: &[u8],
        path_and_query: &[u8],
        body_digest: [u8; 32],
        transport: ProofTransport,
        proxy_expires_at: DateTime<Utc>,
        authenticated_client_peer: AuthenticatedClientPeer,
        nonce_claim: [u8; 32],
    ) -> Self {
        let assertion = assertion.into();
        let assertion_digest: [u8; 32] = Sha256::digest(assertion.as_bytes()).into();
        Self::from_trusted_proxy(
            authorization_domain,
            assertion,
            assertion_digest,
            method,
            authority,
            path_and_query,
            body_digest,
            transport,
            proxy_expires_at,
            TrustedProxyNonceClaim::new(nonce_claim, proxy_expires_at),
            AssertionTransportProfile::TrustedProxyHmacV2,
            Some(authenticated_client_peer),
        )
    }

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
        profile: AssertionTransportProfile,
        authenticated_client_peer: Option<AuthenticatedClientPeer>,
    ) -> Self {
        let peer_key = authenticated_client_peer
            .as_ref()
            .map(AuthenticatedClientPeer::admission_key)
            .map(<[u8; 32]>::as_slice)
            .unwrap_or_default();
        let (request_domain, transport_profile) = match profile {
            AssertionTransportProfile::TrustedProxyHmacV1 => (
                b"buzz:nip-fi:trusted-proxy-request:v1".as_slice(),
                b"trusted-proxy-hmac-v1".as_slice(),
            ),
            AssertionTransportProfile::TrustedProxyHmacV2 => (
                b"buzz:nip-fi:trusted-proxy-request:v2".as_slice(),
                b"trusted-proxy-hmac-v2".as_slice(),
            ),
        };
        let request_fingerprint = if authenticated_client_peer.is_some() {
            framed_fingerprint(
                request_domain,
                &[
                    authorization_domain.as_uuid().as_bytes(),
                    method,
                    authority,
                    path_and_query,
                    &body_digest,
                    &[proof_transport_code(transport)],
                    peer_key,
                ],
            )
        } else {
            framed_fingerprint(
                request_domain,
                &[
                    authorization_domain.as_uuid().as_bytes(),
                    method,
                    authority,
                    path_and_query,
                    &body_digest,
                    &[proof_transport_code(transport)],
                ],
            )
        };
        let transport_context_fingerprint = if authenticated_client_peer.is_some() {
            framed_fingerprint(
                b"buzz:nip-fi:assertion-transport:v2",
                &[
                    transport_profile,
                    authorization_domain.as_uuid().as_bytes(),
                    authority,
                    &[proof_transport_code(transport)],
                    peer_key,
                ],
            )
        } else {
            framed_fingerprint(
                b"buzz:nip-fi:assertion-transport:v1",
                &[
                    transport_profile,
                    authorization_domain.as_uuid().as_bytes(),
                    authority,
                    &[proof_transport_code(transport)],
                ],
            )
        };
        Self {
            authorization_domain,
            assertion,
            assertion_digest,
            request_fingerprint,
            transport_context_fingerprint,
            transport,
            proxy_expires_at,
            nonce_claim,
            profile,
            authenticated_client_peer,
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

    /// Authenticated end-client peer key, when the proxy used the v2 profile.
    pub const fn authenticated_client_peer(&self) -> Option<&AuthenticatedClientPeer> {
        self.authenticated_client_peer.as_ref()
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
