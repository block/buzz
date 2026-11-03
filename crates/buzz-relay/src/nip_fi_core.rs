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

/// The request `Host`, if present and readable.
fn request_host(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
}

/// Resolve the community served at the request `Host` from a static map.
///
/// An absent, unreadable, or unmapped Host is `authorization_unavailable`.
/// This makes mapped and unmapped Hosts distinguishable (401/403 vs 503) in
/// enforce mode, unlike the tenant binder's generic 404; the spec requires it,
/// and Hosts are DNS-visible anyway.
pub(crate) fn resolve_community<'a>(
    headers: &HeaderMap,
    communities: &'a NipFiCommunities,
) -> Result<&'a std::sync::Arc<CommunityBinding>, DenialClass> {
    request_host(headers)
        .and_then(|host| communities.resolve(host))
        .ok_or(DenialClass::AuthorizationUnavailable)
}

/// A community resolved for one enforce admission.
#[derive(Clone)]
pub(crate) struct ResolvedCommunity {
    binding: std::sync::Arc<CommunityBinding>,
    deadline: AdmissionDeadline,
}

impl ResolvedCommunity {
    /// The deadline admission must re-check after its own awaits.
    pub(crate) fn deadline(&self) -> AdmissionDeadline {
        self.deadline.clone()
    }

    /// The shadow `community` label: the configured URI for an `env`
    /// binding, whose set is bounded by config, else `bound`, since the
    /// database's host set is unbounded.
    pub(crate) fn shadow_label(&self) -> metrics::SharedString {
        if self.deadline.0.is_none() {
            self.binding.expected_aud().to_owned().into()
        } else {
            metrics::SharedString::const_str(crate::nip_fi_shadow::BOUND)
        }
    }
}

impl std::ops::Deref for ResolvedCommunity {
    type Target = CommunityBinding;
    fn deref(&self) -> &CommunityBinding {
        &self.binding
    }
}

/// The point past which admitting on a resolved community is
/// `authorization_unavailable`: the database snapshot's staleness deadline.
/// A static `env` binding never goes stale.
#[derive(Clone, Default)]
pub(crate) struct AdmissionDeadline(Option<crate::nip_fi_resolver::FreshUntil>);

impl AdmissionDeadline {
    /// Run immediately before admitting.
    pub(crate) fn check(&self) -> Result<(), DenialClass> {
        self.0.as_ref().map_or(Ok(()), |fresh| fresh.check())
    }
}

/// The relay's community source (`BUZZ_NIP_FI_COMMUNITY_SOURCE`): the
/// static `env` map, or the database resolver.
#[derive(Clone, Copy)]
pub(crate) enum Communities<'a> {
    Env(&'a NipFiCommunities),
    Database(&'a std::sync::Arc<crate::nip_fi_resolver::DatabaseResolver>),
    /// The `database` source with no resolver built: every host is
    /// `authorization_unavailable`.
    Unavailable,
}

impl Communities<'_> {
    /// Resolve the community at the request `Host` — enforce-mode admission
    /// step 1, before any evidence is examined (NIP-FI.md:266-268). Called
    /// once per admission; the returned binding is what verification uses.
    pub(crate) async fn resolve(
        self,
        headers: &HeaderMap,
    ) -> Result<ResolvedCommunity, DenialClass> {
        match self {
            Self::Env(map) => resolve_community(headers, map).map(|b| ResolvedCommunity {
                binding: std::sync::Arc::clone(b),
                deadline: AdmissionDeadline::default(),
            }),
            Self::Database(resolver) => {
                let host = request_host(headers).ok_or(DenialClass::AuthorizationUnavailable)?;
                let (binding, fresh) = resolver.resolve(host).await?;
                Ok(ResolvedCommunity {
                    binding,
                    deadline: AdmissionDeadline(Some(fresh)),
                })
            }
            Self::Unavailable => Err(DenialClass::AuthorizationUnavailable),
        }
    }
}

impl<'a> From<&'a NipFiCommunities> for Communities<'a> {
    fn from(map: &'a NipFiCommunities) -> Self {
        Self::Env(map)
    }
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

/// Whether the deny set holds `(iss, k)` or, for an agent assertion,
/// `(iss, owner)` (NIP-FI.md, admission step 7).
pub(crate) fn assertion_denied(
    deny_map: &impl crate::nip_fi_http::HttpDenyMap,
    assertion: &VerifiedAssertion,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let issuer = assertion.identity().issuer();
    [assertion.asserted_key(), assertion.owner()]
        .into_iter()
        .flatten()
        .any(|key| deny_map.is_denied(issuer, &key, now))
}

/// The NIP-FI facts a live session is registered under: its admitting issuer
/// and, for an agent assertion, the verified owner.
#[derive(Clone)]
pub(crate) struct NipFiSession {
    issuer: String,
    owner: Option<[u8; 32]>,
}

impl NipFiSession {
    pub(crate) fn new(issuer: impl Into<String>, owner: Option<[u8; 32]>) -> Self {
        Self {
            issuer: issuer.into(),
            owner,
        }
    }

    pub(crate) fn from_assertion(assertion: &VerifiedAssertion) -> Self {
        Self::new(
            assertion.identity().issuer(),
            assertion.owner().map(|owner| owner.to_bytes()),
        )
    }

    /// An admin disconnect of `(issuer, target)` reaches this session when
    /// its proven key or its recorded owner is `target`.
    pub(crate) fn matches(&self, issuer: &str, proven: &[u8], target: &[u8]) -> bool {
        self.issuer == issuer
            && (proven == target || self.owner.is_some_and(|owner| owner[..] == *target))
    }
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

    /// The issuer and key ID of [`assertion_signing_key`].
    pub(crate) const TEST_ISSUER: &str = "https://issuer.example";
    pub(crate) const TEST_KID: &str = "test-key-1";

    /// Ephemeral signing material shared by the test signer and verifier.
    /// Never persist a private key fixture in source or on disk.
    pub(crate) fn assertion_signing_key(
    ) -> &'static (jsonwebtoken::EncodingKey, jsonwebtoken::jwk::JwkSet) {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
        static KEY: std::sync::OnceLock<(jsonwebtoken::EncodingKey, jsonwebtoken::jwk::JwkSet)> =
            std::sync::OnceLock::new();
        KEY.get_or_init(|| {
            let rng = ring::rand::SystemRandom::new();
            let algorithm = &ECDSA_P256_SHA256_FIXED_SIGNING;
            let der = EcdsaKeyPair::generate_pkcs8(algorithm, &rng).expect("generate test key");
            let pair = EcdsaKeyPair::from_pkcs8(algorithm, der.as_ref(), &rng)
                .expect("parse generated test key");
            // P-256 public keys use uncompressed SEC1: 0x04 || x || y.
            let public = pair.public_key().as_ref();
            assert_eq!(public.len(), 65);
            assert_eq!(public[0], 4);
            let jwks = serde_json::from_value(serde_json::json!({
                "keys": [{
                    "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256",
                    "kid": TEST_KID,
                    "x": URL_SAFE_NO_PAD.encode(&public[1..33]),
                    "y": URL_SAFE_NO_PAD.encode(&public[33..65])
                }]
            }))
            .expect("valid generated JWKS");
            (jsonwebtoken::EncodingKey::from_ec_der(der.as_ref()), jwks)
        })
    }

    /// The real verifier trusting [`assertion_signing_key`] for [`TEST_ISSUER`]
    /// at [`TEST_COMMUNITY_URI`], under `token_class`.
    pub(crate) fn assertion_verifier(
        token_class: buzz_auth::TokenClass,
    ) -> buzz_auth::FederatedAssertionVerifier<buzz_auth::StaticIssuerKeySource> {
        use buzz_auth::{
            AssertionKeySet, FederatedAssertionVerifier, FreshnessClass, IssuerPolicy,
            IssuerRegistry, StaticIssuerKeySource,
        };
        use jsonwebtoken::Algorithm;

        let jwks = assertion_signing_key().1.clone();
        let hard_deadline = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let key_set = AssertionKeySet::new_for_test(TEST_ISSUER.to_owned(), 1, jwks, hard_deadline)
            .expect("valid test key set");
        let jwks_contract = buzz_auth::JwksSourceContract::new(
            format!("{TEST_ISSUER}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid jwks contract");
        let policy = IssuerPolicy::new(
            TEST_ISSUER.to_owned(),
            vec![TEST_COMMUNITY_URI.to_owned()],
            token_class,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            3600,
            None,
            jwks_contract,
        )
        .expect("valid issuer policy");
        let mut registry = IssuerRegistry::new();
        registry.insert(policy);
        FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([key_set]))
    }

    /// An `at+jwt` token class whose resource-owner subjects carry
    /// `sub_type: "user"`.
    pub(crate) fn at_jwt_class() -> buzz_auth::TokenClass {
        buzz_auth::TokenClass::AccessTokenAtJwt {
            subject_class: buzz_auth::SubjectClassContract::new(
                "sub_type".to_owned(),
                vec!["user".to_owned()],
                vec!["client".to_owned()],
                buzz_auth::ClientSubjectPosture::Reject,
            )
            .expect("valid subject class"),
        }
    }

    /// Sign an assertion with an explicit `typ` and raw `nostr_owner_pubkey`.
    /// It also carries claims satisfying [`at_jwt_class`] (`client_id`,
    /// resource-owner `sub_type`), so under an `at+jwt` policy only `typ`
    /// decides; dedicated policies ignore them.
    pub(crate) fn sign_assertion(
        typ: &str,
        pubkey_hex: &str,
        owner: Option<serde_json::Value>,
    ) -> String {
        use jsonwebtoken::{Algorithm, Header};
        let now = chrono::Utc::now().timestamp();
        let mut claims = serde_json::json!({
            "iss": TEST_ISSUER,
            "aud": TEST_COMMUNITY_URI,
            "iat": now,
            "exp": now + 600,
            "sub": "test-subject",
            "nostr_pubkey": pubkey_hex,
            "client_id": "test-client",
            "sub_type": "user",
        });
        if let Some(owner) = owner {
            claims[buzz_auth::NOSTR_OWNER_PUBKEY_CLAIM] = owner;
        }
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(TEST_KID.to_owned());
        header.typ = Some(typ.to_owned());
        let key = &assertion_signing_key().0;
        jsonwebtoken::encode(&header, &claims, key).expect("sign assertion")
    }

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
        (**any_host(aud).resolve(TEST_HOST).expect("served")).clone()
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
        CommunityBinding::clone(
            communities()
                .resolve(super::test_support::TEST_HOST)
                .expect("mapped"),
        )
    }

    /// An agent assertion for `agent` owned by `owner`, from `test-issuer`.
    fn agent_assertion(agent: nostr::PublicKey, owner: nostr::PublicKey) -> VerifiedAssertion {
        VerifiedAssertion::for_test(
            Some(agent),
            vec![chrono::Utc::now() + chrono::Duration::seconds(300)],
        )
        .with_owner_for_test(owner)
    }

    fn deny_map_denying(key: &nostr::PublicKey) -> buzz_auth::NipFiDenyMap {
        let map = buzz_auth::NipFiDenyMap::new(
            16,
            vec![buzz_auth::IssuerCapacity {
                issuer: "test-issuer".to_owned(),
                capacity: 16,
            }],
        );
        let now = chrono::Utc::now();
        map.merge_cross_pod_deny("test-issuer", key, now + chrono::Duration::seconds(60), now);
        map
    }

    #[test]
    fn deny_entry_for_the_owner_denies_its_agent_assertion() {
        let agent = nostr::Keys::generate().public_key();
        let owner = nostr::Keys::generate().public_key();
        let unrelated = nostr::Keys::generate().public_key();
        let assertion = agent_assertion(agent, owner);
        let now = chrono::Utc::now();
        assert!(assertion_denied(&deny_map_denying(&owner), &assertion, now));
        assert!(assertion_denied(&deny_map_denying(&agent), &assertion, now));
        assert!(!assertion_denied(
            &deny_map_denying(&unrelated),
            &assertion,
            now
        ));
    }

    #[test]
    fn owner_deny_entry_does_not_reach_an_ownerless_assertion() {
        let key = nostr::Keys::generate().public_key();
        let other = nostr::Keys::generate().public_key();
        let assertion = VerifiedAssertion::for_test(
            Some(key),
            vec![chrono::Utc::now() + chrono::Duration::seconds(300)],
        );
        assert!(!assertion_denied(
            &deny_map_denying(&other),
            &assertion,
            chrono::Utc::now()
        ));
    }

    #[test]
    fn session_matches_disconnect_of_its_key_or_owner_under_its_issuer() {
        let (agent, owner, other) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let session = NipFiSession::new("test-issuer", Some(owner));
        assert!(session.matches("test-issuer", &agent, &agent));
        assert!(session.matches("test-issuer", &agent, &owner));
        assert!(!session.matches("test-issuer", &agent, &other));
        assert!(!session.matches("other-issuer", &agent, &owner));
        let human = NipFiSession::new("test-issuer", None);
        assert!(!human.matches("test-issuer", &agent, &owner));
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
