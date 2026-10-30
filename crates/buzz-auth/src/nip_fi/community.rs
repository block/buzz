//! The community an assertion is presented to (NIP-FI.md:118-128, :193-197).
//!
//! A [`CommunityBinding`] is the deployment-policy pair the relay resolves from
//! the request `Host` before verification: the community's exact expected
//! `aud`, and the issuers authorized to assert identities for it. The verifier
//! checks the allowlist before any issuer-specific dependency (JWKS) and
//! matches `aud` exactly against this one value, so an issuer trusted for one
//! community can never mint an assertion accepted by another.

use std::collections::BTreeSet;

/// One community's assertion-acceptance policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityBinding {
    expected_aud: String,
    authorized_issuers: BTreeSet<String>,
}

/// Why a [`CommunityBinding`] could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommunityBindingError {
    /// The expected `aud` was empty.
    #[error("empty expected audience")]
    EmptyAudience,
    /// No issuer, or an empty issuer string, was authorized.
    #[error("empty authorized issuer set")]
    EmptyIssuers,
}

impl CommunityBinding {
    /// Bind a community's exact `aud` to its non-empty issuer allowlist.
    pub fn new(
        expected_aud: String,
        authorized_issuers: impl IntoIterator<Item = String>,
    ) -> Result<Self, CommunityBindingError> {
        if expected_aud.is_empty() {
            return Err(CommunityBindingError::EmptyAudience);
        }
        let authorized_issuers: BTreeSet<String> = authorized_issuers.into_iter().collect();
        if authorized_issuers.is_empty() || authorized_issuers.iter().any(String::is_empty) {
            return Err(CommunityBindingError::EmptyIssuers);
        }
        Ok(Self {
            expected_aud,
            authorized_issuers,
        })
    }

    /// The exact `aud` an assertion for this community must carry.
    pub fn expected_aud(&self) -> &str {
        &self.expected_aud
    }

    /// Whether `issuer` (exact bytes) may assert identities for this community.
    pub fn authorizes(&self, issuer: &str) -> bool {
        self.authorized_issuers.contains(issuer)
    }

    /// The authorized issuers, in canonical order.
    pub fn authorized_issuers(&self) -> impl Iterator<Item = &str> {
        self.authorized_issuers.iter().map(String::as_str)
    }
}
