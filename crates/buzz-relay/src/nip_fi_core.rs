//! Shared NIP-FI assertion evaluation for every relay ingress adapter.
//!
//! The HTTP guard, WebSocket upgrade, and HTTP admission each decide *when*
//! to evaluate an attached assertion (mode short-circuits, exemptions, NIP-98
//! ordering) and what to do with the result. This module owns the part they
//! must agree on: extracting the `Nostr-Federated-Identity` bearer, verifying
//! it, mapping failures to a [`DenialClass`], rendering the exact HTTP denial,
//! and the key-pairing predicate. [FI-TRACE-AUTHORITY-UNIFORM]
//!
//! The admin disconnect route also uses [`extract_bearer_token`] and
//! [`http_denial`] for its command JWT, so its transport failures render
//! exactly like an assertion's.

use axum::{
    body::Body,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
};
use buzz_auth::{
    CommunityBinding, DenialClass, VerifiedAssertion, VerifierError, VerifyAssertion,
    CLIENT_ATTACHED_HEADER,
};

use crate::nip_fi_config::NipFiCommunities;

// ── Assertion evaluation ──────────────────────────────────────────────────────

/// Why an attached assertion was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssertionRejection {
    /// The header was absent or malformed. [FI-TRACE-TRANSPORT-CLOSED]
    Transport(DenialClass),
    /// No verifier is constructed yet (startup race); fail closed.
    VerifierUnavailable,
    /// The verifier rejected the token.
    Verifier(VerifierError),
}

impl AssertionRejection {
    /// The public denial class for this rejection.
    pub(crate) const fn denial_class(self) -> DenialClass {
        match self {
            Self::Transport(class) => class,
            Self::VerifierUnavailable => DenialClass::AuthorizationUnavailable,
            Self::Verifier(err) => err.denial_class(),
        }
    }
}

/// Resolve the community served at the request `Host` — enforce-mode
/// admission step 1, before any evidence is examined (NIP-FI.md:266-268).
///
/// An absent, unreadable, or unmapped Host is `authorization_unavailable`.
/// This makes mapped and unmapped Hosts distinguishable (401/403 vs 503) in
/// enforce mode, unlike the tenant binder's generic 404; the spec requires it,
/// and Hosts are DNS-visible anyway.
pub(crate) fn resolve_community<'a>(
    headers: &HeaderMap,
    communities: &'a NipFiCommunities,
) -> Result<&'a CommunityBinding, DenialClass> {
    headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|host| communities.resolve(host))
        .ok_or(DenialClass::AuthorizationUnavailable)
}

/// Extract and verify the attached assertion for `community`.
///
/// Transport extraction runs before the verifier-presence check, so a
/// malformed header is `evidence_rejected` even while no verifier exists.
pub(crate) fn evaluate_attached_assertion(
    headers: &HeaderMap,
    community: &CommunityBinding,
    verifier: Option<&dyn VerifyAssertion>,
) -> Result<VerifiedAssertion, AssertionRejection> {
    let token = extract_bearer_token(headers).map_err(AssertionRejection::Transport)?;
    let verifier = verifier.ok_or(AssertionRejection::VerifierUnavailable)?;
    verifier
        .verify_assertion(token, community)
        .map_err(AssertionRejection::Verifier)
}

/// The assertion's `nostr_pubkey` claim equals the key the client proved.
/// A claimless assertion never matches. [FI-INV-05]
pub(crate) fn asserted_key_matches(
    assertion: &VerifiedAssertion,
    proven_pubkey: nostr::PublicKey,
) -> bool {
    assertion.asserted_key() == Some(proven_pubkey)
}

// ── Transport extraction ──────────────────────────────────────────────────────

/// Extract the single `Bearer <token>` from the `Nostr-Federated-Identity`
/// header.
///
/// Rejects all forms the spec prohibits:
/// - Absent → `MissingEvidence`
/// - Repeated (multiple header values) → `EvidenceRejected`
/// - Comma-combined (`,` in a single value) → `EvidenceRejected`
/// - Empty after `Bearer ` stripping → `EvidenceRejected`
/// - Non-`Bearer ` prefix → `EvidenceRejected`
/// - Whitespace in the token (after scheme) → `EvidenceRejected`
///
/// [FI-TRACE-TRANSPORT-CLOSED]
pub(crate) fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, DenialClass> {
    let mut values = headers.get_all(CLIENT_ATTACHED_HEADER).iter();
    let first = match values.next() {
        Some(v) => v,
        None => return Err(DenialClass::MissingEvidence),
    };
    // Repeated header fields deny. [FI-TRACE-TRANSPORT-CLOSED]
    if values.next().is_some() {
        return Err(DenialClass::EvidenceRejected);
    }
    let raw = first.to_str().map_err(|_| DenialClass::EvidenceRejected)?;
    // Comma-combined values deny.
    if raw.contains(',') {
        return Err(DenialClass::EvidenceRejected);
    }
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or(DenialClass::EvidenceRejected)?;
    // Empty or whitespace-containing token denies.
    if token.is_empty() || token.contains(ascii_whitespace) {
        return Err(DenialClass::EvidenceRejected);
    }
    Ok(token)
}

fn ascii_whitespace(c: char) -> bool {
    c.is_ascii_whitespace()
}

// ── HTTP denial response ──────────────────────────────────────────────────────

/// Build the exact HTTP denial response for the given class.
///
/// The response contract is fixed by NIP-FI.md rejection table:
/// - Status, Content-Type, WWW-Authenticate (for 401), and body bytes are the
///   closed contract.  No other fields are added that depend on the private
///   condition. [FI-TRACE-DENIAL-ORACLE]
pub(crate) fn http_denial(class: DenialClass) -> Response<Body> {
    // Every status and header value is a fixed constant, so the fallbacks are
    // unreachable; they keep the response a denial rather than panicking.
    let status =
        StatusCode::from_u16(class.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", class.content_type());
    if let Some(challenge) = class.www_authenticate() {
        builder = builder.header("WWW-Authenticate", challenge);
    }
    builder
        .body(Body::from(class.http_body()))
        .unwrap_or_else(|_| status.into_response())
}

/// Fixtures for tests that drive an enforce-mode adapter against one mapped
/// community.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::nip_fi_config::NipFiCommunities;

    /// The test community's Host and canonical URI.
    pub(crate) const TEST_HOST: &str = "relay.example";
    pub(crate) const TEST_COMMUNITY_URI: &str = "https://relay.example";

    /// One community at [`TEST_COMMUNITY_URI`] authorizing `issuers`.
    pub(crate) fn communities_for(issuers: &[&str]) -> NipFiCommunities {
        NipFiCommunities::for_test(TEST_COMMUNITY_URI, issuers)
    }

    /// The test community authorizing a placeholder issuer, for verifier
    /// doubles that ignore the binding.
    pub(crate) fn communities() -> NipFiCommunities {
        communities_for(&["https://issuer.test"])
    }

    /// Every issuer the crate's enforce-mode fixtures configure.
    const FIXTURE_ISSUERS: &[&str] = &[
        "https://issuer.example",
        "https://issuer.test",
        "https://idp.test.example.com",
        "https://idp-b.test.example.com",
        "https://nip-fi-deny-test.example.com",
        "https://git-pack-test.issuer.invalid",
        "https://nip-fi-settings-test.invalid",
    ];

    /// One community expecting `aud`, authorizing every fixture issuer, and
    /// served at every Host — for handler fixtures on per-test unique Hosts.
    pub(crate) fn any_host(aud: &str) -> NipFiCommunities {
        NipFiCommunities::for_test_any_host(aud, FIXTURE_ISSUERS)
    }

    /// The binding a fixture community expecting `aud` presents.
    pub(crate) fn binding(aud: &str) -> CommunityBinding {
        any_host(aud).resolve(TEST_HOST).expect("served").clone()
    }

    /// Request headers addressed to [`TEST_HOST`].
    pub(crate) fn host_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            axum::http::HeaderValue::from_static(TEST_HOST),
        );
        headers
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::test_support::communities;
    use super::*;
    use axum::http::HeaderValue;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn community() -> CommunityBinding {
        communities()
            .resolve(super::test_support::TEST_HOST)
            .expect("mapped")
            .clone()
    }

    /// Verifier returning a fixed result and counting calls.
    ///
    /// Lives here, next to the shared evaluator, so the crate has one NIP-FI
    /// verifier fixture. Shared with the `nip_fi_http`, `nip_fi_upgrade`, and
    /// `router` tests.
    pub(crate) struct ScriptedVerifier {
        result: Result<Option<nostr::PublicKey>, VerifierError>,
        calls: AtomicUsize,
    }

    impl ScriptedVerifier {
        /// A verifier that answers every call with `result`, where `Ok(key)`
        /// becomes an assertion for `key` expiring in an hour.
        pub(crate) fn new(result: Result<Option<nostr::PublicKey>, VerifierError>) -> Self {
            Self {
                result,
                calls: AtomicUsize::new(0),
            }
        }

        /// How many times `verify_assertion` has run.
        pub(crate) fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl VerifyAssertion for ScriptedVerifier {
        fn verify_assertion(
            &self,
            _token: &str,
            _community: &CommunityBinding,
        ) -> Result<VerifiedAssertion, VerifierError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.map(|key| {
                VerifiedAssertion::for_test(
                    key,
                    vec![chrono::Utc::now() + chrono::Duration::hours(1)],
                )
            })
        }
    }

    fn headers(value: &'static str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CLIENT_ATTACHED_HEADER, HeaderValue::from_static(value));
        h
    }

    #[test]
    fn missing_header_is_transport_missing_evidence() {
        let v = ScriptedVerifier::new(Ok(None));
        assert_eq!(
            evaluate_attached_assertion(&HeaderMap::new(), &community(), Some(&v)).unwrap_err(),
            AssertionRejection::Transport(DenialClass::MissingEvidence)
        );
    }

    #[test]
    fn transport_rejection_precedes_missing_verifier() {
        // A comma-joined value is otherwise well-formed, so only the comma
        // check separates it from the missing-verifier outcome.
        for value in ["junk", "Bearer a.b.c,d.e.f"] {
            let err = evaluate_attached_assertion(&headers(value), &community(), None).unwrap_err();
            assert_eq!(
                err,
                AssertionRejection::Transport(DenialClass::EvidenceRejected),
                "{value}"
            );
        }
    }

    #[test]
    fn missing_verifier_is_authorization_unavailable() {
        let err =
            evaluate_attached_assertion(&headers("Bearer a.b.c"), &community(), None).unwrap_err();
        assert_eq!(err, AssertionRejection::VerifierUnavailable);
        assert_eq!(err.denial_class(), DenialClass::AuthorizationUnavailable);
    }

    #[test]
    fn verifier_error_keeps_its_denial_class() {
        for err in [
            VerifierError::KeySourceUnavailable,
            VerifierError::InvalidSignatureOrClaims,
        ] {
            let v = ScriptedVerifier::new(Err(err));
            let rejection =
                evaluate_attached_assertion(&headers("Bearer a.b.c"), &community(), Some(&v))
                    .unwrap_err();
            assert_eq!(rejection, AssertionRejection::Verifier(err));
            assert_eq!(rejection.denial_class(), err.denial_class());
        }
    }

    #[test]
    fn asserted_key_matches_only_the_proven_key() {
        let proven = nostr::Keys::generate().public_key();
        let other = nostr::Keys::generate().public_key();
        let at = |k| {
            VerifiedAssertion::for_test(k, vec![chrono::Utc::now() + chrono::Duration::hours(1)])
        };
        assert!(asserted_key_matches(&at(Some(proven)), proven));
        assert!(!asserted_key_matches(&at(Some(other)), proven));
        assert!(!asserted_key_matches(&at(None), proven));
    }
}
