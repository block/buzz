//! Behavior tests for the NIP-FI canonical assertion verifier and contracts
//! (PR 1). Exercises the exact-wire-text denial contract, deterministic
//! contract IDs, token-class enforcement including ID-token denial, and
//! multi-issuer `(iss, sub)` selection, against real ES256-signed assertions.
//!
//! In-crate unit tests: the crate-owned [`StaticIssuerKeySource`] and the
//! crate-private `AssertionKeySet::new` constructor are the only way to supply
//! key material to the verifier, and both are `cfg(test)`-only — reachable
//! here because this module compiles inside `buzz_auth` under `cargo test`, but
//! not exposed to any dependent crate under any Cargo feature. That keeps the
//! issuer→JWKS authority entirely crate-owned.

use super::*;
use crate::nip_fi::{IssuerPolicyError, SubjectClassContract, CLIENT_ATTACHED_HEADER};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{json, Value};

// A fixed P-256 test key (PKCS#8 PEM) and its public JWK coordinates.
const TEST_EC_PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
-----END PRIVATE KEY-----\n";
const TEST_JWK_X: &str = "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI";
const TEST_JWK_Y: &str = "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA";
const TEST_KID: &str = "test-key-1";
const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "https://relay.example";
/// A canonical lowercase-hex nostr pubkey for tokens that are not testing
/// the nostr_pubkey claim specifically. Spec v2 requires the claim unconditionally.
const TEST_NOSTR_PUBKEY: &str = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

/// A canonical JWKS contract for the default test issuer. Used wherever a
/// `JwksSourceContract` is required but JWKS behavior is not under test.
fn test_jwks_contract() -> crate::nip_fi::jwks::JwksSourceContract {
    crate::nip_fi::jwks::JwksSourceContract::new(
        format!("{}/.well-known/jwks.json", ISSUER),
        300,
        3600,
    )
    .expect("valid test contract")
}

// A second, independent P-256 key: issuer B's real signing key, used to prove
// that a token signed by B and claiming `iss=A` cannot mint an A identity.
const TEST_EC_PKCS8_PEM_B: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgKcmDf3+zDWyC96/X\n\
Gv8aYK552uF5aE6nXKzxAfl4fSWhRANCAATf0ccbp1c4mMd6WvSuliv5ZAS8iIWL\n\
Ne2tqOfFa0hRpa41DANab1/EuDGi7PtIo8xSYwkaoib1MAJlfLvRMjQA\n\
-----END PRIVATE KEY-----\n";
const TEST_JWK_X_B: &str = "39HHG6dXOJjHelr0rpYr-WQEvIiFizXtrajnxWtIUaU";
const TEST_JWK_Y_B: &str = "rjUMA1pvX8S4MaLs-0ijzFJjCRqiJvUwAmV8u9EyNAA";

// A trusted [`StaticIssuerKeySource`] is used throughout, standing in for
// PR 3's JWKS runtime. Because the key-source trait is sealed, an external
// crate cannot implement its own source at all — the authority-construction
// seam is closed, and the only way to exercise the verifier is this
// crate-owned source. It returns only a snapshot bound to the exact issuer
// requested — the invariant the real source guarantees.
fn test_jwks(kid: &str) -> JwkSet {
    jwks_with_coords(kid, TEST_JWK_X, TEST_JWK_Y)
}

fn jwks_with_coords(kid: &str, x: &str, y: &str) -> JwkSet {
    serde_json::from_value(json!({
        "keys": [{
            "kty": "EC",
            "crv": "P-256",
            "use": "sig",
            "alg": "ES256",
            "kid": kid,
            "x": x,
            "y": y,
        }]
    }))
    .expect("valid JWKS")
}

/// A key-snapshot hard deadline comfortably in the future, so time checks pass
/// and the required-finite-positive-deadline construction succeeds.
fn future_deadline() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::seconds(3600)
}

fn key_set_for(issuer: &str) -> AssertionKeySet {
    AssertionKeySet::new(issuer.to_owned(), 1, test_jwks(TEST_KID), future_deadline())
        .expect("nonzero generation, non-empty issuer")
}

/// A resource-owner/client-subject contract that rejects client-subject tokens.
/// Resource-owner and client-subject subjects are distinguished by a `sub_type`
/// marker claim with disjoint value sets.
fn subject_class_reject() -> SubjectClassContract {
    SubjectClassContract::new(
        "sub_type".to_owned(),
        vec!["user".to_owned()],
        vec!["client".to_owned()],
        ClientSubjectPosture::Reject,
    )
    .expect("valid subject-class contract")
}

fn access_token_policy() -> IssuerPolicy {
    access_token_policy_with(subject_class_reject())
}

fn access_token_policy_with(subject_class: SubjectClassContract) -> IssuerPolicy {
    IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::AccessTokenAtJwt { subject_class },
        FreshnessClass::OfflineJwt,
        vec![Algorithm::ES256],
        60,
        3600,
        None,
        test_jwks_contract(),
    )
    .expect("valid policy")
}

fn dedicated_policy(issuer: &str) -> IssuerPolicy {
    let contract = crate::nip_fi::jwks::JwksSourceContract::new(
        format!("{}/.well-known/jwks.json", issuer.trim_end_matches('/')),
        300,
        3600,
    )
    .expect("valid test contract");
    IssuerPolicy::new(
        issuer.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        vec![Algorithm::ES256],
        60,
        3600,
        None,
        contract,
    )
    .expect("valid policy")
}

fn dedicated_policy_with_audiences(audiences: Vec<String>) -> IssuerPolicy {
    IssuerPolicy::new(
        ISSUER.to_owned(),
        audiences,
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        vec![Algorithm::ES256],
        60,
        3600,
        None,
        test_jwks_contract(),
    )
    .expect("valid policy")
}

fn dedicated_policy_with_algorithms(algorithms: Vec<Algorithm>) -> IssuerPolicy {
    IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        algorithms,
        60,
        3600,
        None,
        test_jwks_contract(),
    )
    .expect("valid policy")
}

fn verifier_with(policy: IssuerPolicy) -> FederatedAssertionVerifier<StaticIssuerKeySource> {
    let mut registry = IssuerRegistry::new();
    let issuer = policy.issuer().to_owned();
    registry.insert(policy);
    FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([key_set_for(&issuer)]))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Mint a signed ES256 assertion with the given `typ`, `kid`, and claims,
/// signed by the default (issuer A) key.
/// Fills in default `iss`/`aud`/`iat`/`exp` if absent.
fn mint(typ: Option<&str>, kid: &str, claims: Value) -> String {
    mint_signed_by(TEST_EC_PKCS8_PEM, typ, kid, claims)
}

/// Mint a signed ES256 assertion with an explicit signing key (PKCS#8 PEM).
fn mint_signed_by(pkcs8_pem: &str, typ: Option<&str>, kid: &str, mut claims: Value) -> String {
    {
        let obj = claims.as_object_mut().expect("claims object");
        obj.entry("iss").or_insert(json!(ISSUER));
        obj.entry("aud").or_insert(json!(AUDIENCE));
        obj.entry("iat").or_insert(json!(now()));
        obj.entry("exp").or_insert(json!(now() + 600));
        // Spec v2 requires nostr_pubkey unconditionally; inject a canonical
        // test pubkey so tokens that test other behaviours pass the claim check.
        obj.entry(NOSTR_PUBKEY_CLAIM)
            .or_insert(json!(TEST_NOSTR_PUBKEY));
    }
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(kid.to_owned());
    header.typ = typ.map(str::to_owned);
    let key = EncodingKey::from_ec_pem(pkcs8_pem.as_bytes()).expect("valid EC PEM");
    jsonwebtoken::encode(&header, &claims, &key).expect("sign")
}

/// Mint a valid, signed token that deliberately omits `nostr_pubkey`.  Used
/// only to exercise the unconditional missing-claim rejection path; the normal
/// `mint`/`mint_signed_by` helpers always inject the claim via `or_insert` so
/// they cannot produce an absent-claim token.
fn mint_no_pubkey(typ: Option<&str>, kid: &str, mut claims: Value) -> String {
    {
        let obj = claims.as_object_mut().expect("claims object");
        obj.entry("iss").or_insert(json!(ISSUER));
        obj.entry("aud").or_insert(json!(AUDIENCE));
        obj.entry("iat").or_insert(json!(now()));
        obj.entry("exp").or_insert(json!(now() + 600));
        // Intentionally does NOT inject nostr_pubkey.
    }
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(kid.to_owned());
    header.typ = typ.map(str::to_owned);
    let key = EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes()).expect("valid EC PEM");
    jsonwebtoken::encode(&header, &claims, &key).expect("sign")
}

/// A resource-owner `at+jwt` claim set: valid subject-class marker plus client_id.
fn resource_owner_claims() -> Value {
    json!({ "sub": "user-123", "client_id": "app-1", "sub_type": "user" })
}

/// Base64url-encode a JSON string into a JWS segment.
fn b64_segment(json_text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json_text.as_bytes())
}

/// Corrupt a token's signature while keeping it well-formed base64url, so the
/// result exercises post-lookup cryptographic rejection — not the pre-lookup
/// signature-shape gate. The final segment character carries curve-dependent
/// trailing-bit constraints (a flip there can produce invalid base64url), so
/// flip the first signature character instead: a leading character always
/// encodes a full 6-bit value and stays well-formed.
fn tamper_signature(token: &str) -> String {
    let (body, signature) = token.rsplit_once('.').expect("three compact segments");
    let mut chars: Vec<char> = signature.chars().collect();
    let first = &mut chars[0];
    *first = if *first == 'A' { 'B' } else { 'A' };
    format!("{body}.{}", chars.into_iter().collect::<String>())
}

// ---- Happy path ----------------------------------------------------------

#[test]
fn valid_access_token_verifies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(Some("at+jwt"), TEST_KID, resource_owner_claims());
    let assertion = verifier.verify(&token).expect("verifies");
    assert_eq!(assertion.identity().issuer(), ISSUER);
    assert_eq!(assertion.identity().subject(), "user-123");
    // Spec v2: nostr_pubkey is injected by mint() and unconditionally required.
    assert!(assertion.asserted_key().is_some());
    assert!(!assertion.authority_deadlines().is_empty());
    assert_eq!(assertion.assertion_policy_id(), access_token_policy().id());
}

// ---- Token class / typ enforcement, ID-token denial ----------------------

#[test]
fn id_token_denies_even_when_iss_aud_sub_match() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("JWT"),
        TEST_KID,
        json!({ "sub": "user-123", "client_id": "app-1", "sub_type": "user", "nonce": "n" }),
    );
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::TokenTypeRejected);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
}

// ---- Named-compatibility mode removed ------------------------------------

#[test]
fn generic_typ_with_client_id_denies() {
    // A generic/absent-`typ` JWT carrying `client_id`, matching iss/aud/sub, is
    // an OIDC-ID-token shape that a claim-presence "named-compatibility" policy
    // would have wrongly accepted. With that mode removed, no policy accepts a
    // non-`at+jwt`/non-`nip-fi+jwt` type: it denies on exact `typ` mismatch.
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("JWT"),
        TEST_KID,
        json!({ "sub": "user-123", "client_id": "app-1", "sub_type": "user" }),
    );
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::TokenTypeRejected);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
}

#[test]
fn dedicated_class_rejects_at_jwt_typ_and_accepts_nip_fi() {
    let verifier = verifier_with(dedicated_policy(ISSUER));
    let wrong = mint(Some("at+jwt"), TEST_KID, json!({ "sub": "u" }));
    assert_eq!(
        verifier.verify(&wrong).unwrap_err(),
        VerifierError::TokenTypeRejected
    );
    let ok = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    assert!(verifier.verify(&ok).is_ok());
}

#[test]
fn access_token_without_client_id_denies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "user-123", "sub_type": "user" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::ClaimContractRejected
    );
}

// ---- Resource-owner / client-subject classification ----------------------

#[test]
fn resource_owner_marker_verifies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "user-123", "client_id": "app-1", "sub_type": "user" }),
    );
    assert!(verifier.verify(&token).is_ok());
}

#[test]
fn client_subject_marker_denies_under_reject_posture() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "svc-1", "client_id": "app-1", "sub_type": "client" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::ClaimContractRejected
    );
}

#[test]
fn client_subject_marker_verifies_under_accept_non_colliding_posture() {
    let contract = SubjectClassContract::new(
        "sub_type".to_owned(),
        vec!["user".to_owned()],
        vec!["client".to_owned()],
        ClientSubjectPosture::AcceptNonColliding,
    )
    .unwrap();
    let verifier = verifier_with(access_token_policy_with(contract));
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "svc-1", "client_id": "app-1", "sub_type": "client" }),
    );
    assert!(verifier.verify(&token).is_ok());
}

#[test]
fn unclassifiable_subject_marker_denies() {
    // A marker value in neither set cannot be classified as resource-owner or
    // client-subject, so the token is ambiguous and denies.
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "user-123", "client_id": "app-1", "sub_type": "mystery" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::ClaimContractRejected
    );
}

#[test]
fn subject_class_contract_rejects_overlapping_value_sets() {
    let err = SubjectClassContract::new(
        "sub_type".to_owned(),
        vec!["user".to_owned(), "shared".to_owned()],
        vec!["shared".to_owned()],
        ClientSubjectPosture::Reject,
    )
    .unwrap_err();
    assert_eq!(err, IssuerPolicyError::NonExclusiveSubjectClass);
}

// ---- Algorithm / key rejection -------------------------------------------

#[test]
fn hs256_symmetric_algorithm_denies() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = b64.encode(json!({"alg":"HS256","kid":TEST_KID,"typ":"at+jwt"}).to_string());
    let payload = b64.encode(
        json!({"iss":ISSUER,"aud":AUDIENCE,"sub":"u","client_id":"a","iat":now(),"exp":now()+600})
            .to_string(),
    );
    let token = format!("{header}.{payload}.AAAA");
    let verifier = verifier_with(access_token_policy());
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::UnsupportedAlgorithm
    );
}

#[test]
fn alg_none_denies() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = b64.encode(json!({"alg":"none","kid":TEST_KID,"typ":"at+jwt"}).to_string());
    let payload = b64.encode(json!({"iss":ISSUER,"aud":AUDIENCE,"sub":"u"}).to_string());
    let token = format!("{header}.{payload}.");
    let verifier = verifier_with(access_token_policy());
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::UnsupportedAlgorithm
    );
}

#[test]
fn unknown_kid_denies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        "other-kid",
        json!({ "sub": "u", "client_id": "a" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::AmbiguousKeyId
    );
}

#[test]
fn tampered_signature_denies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a" }),
    );
    // A well-formed but cryptographically wrong signature: post-lookup crypto
    // rejection, not the pre-lookup signature-shape gate.
    let token = tamper_signature(&token);
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidSignatureOrClaims
    );
}

#[test]
fn wrong_audience_denies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "aud": "https://other.example" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidSignatureOrClaims
    );
}

#[test]
fn key_restricted_to_encrypt_key_ops_denies() {
    // A matching `kid` whose JWK restricts `key_ops` to `encrypt` cannot verify
    // a signature (NIP-FI.md:166-169 rejects incompatible JWK usage). Absent a
    // `key_ops` check the signature would validate under the same EC key.
    let jwks: JwkSet = serde_json::from_value(json!({
        "keys": [{
            "kty": "EC",
            "crv": "P-256",
            "key_ops": ["encrypt"],
            "alg": "ES256",
            "kid": TEST_KID,
            "x": TEST_JWK_X,
            "y": TEST_JWK_Y,
        }]
    }))
    .expect("valid JWKS");
    let key_set =
        AssertionKeySet::new(ISSUER.to_owned(), 1, jwks, future_deadline()).expect("valid key set");
    let mut registry = IssuerRegistry::new();
    registry.insert(access_token_policy());
    let verifier = FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([key_set]));
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "sub_type": "user" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidKey
    );
}

// ---- Algorithm ↔ key family/curve binding (P1 #1) ------------------------
//
// The optional JWK `alg` is advisory; the key material (`kty`/`crv`) is what
// signs. `validate_jwk` runs before signature verification, so a JWK whose
// declared `alg` matches the token but whose material is a different family or
// curve must deny as `InvalidKey` — a cross-family/cross-curve substitution
// can never mint a `VerifiedAssertion` (NIP-FI.md:166-171).

fn install_jwk_for(
    policy_algorithms: Vec<Algorithm>,
    jwk: Value,
) -> FederatedAssertionVerifier<StaticIssuerKeySource> {
    let jwks: JwkSet = serde_json::from_value(json!({ "keys": [jwk] })).expect("valid JWKS");
    let key_set =
        AssertionKeySet::new(ISSUER.to_owned(), 1, jwks, future_deadline()).expect("valid key set");
    let mut registry = IssuerRegistry::new();
    registry.insert(dedicated_policy_with_algorithms(policy_algorithms));
    FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([key_set]))
}

#[test]
fn es256_token_against_p384_curve_material_denies() {
    // Carl's exploit: a JWK declaring `crv=P-384, alg=ES256` over valid P-256
    // coordinates. The advisory `alg` matches the ES256 token, but the curve is
    // wrong, so the key material is inadmissible.
    let verifier = install_jwk_for(
        vec![Algorithm::ES256],
        json!({
            "kty": "EC",
            "crv": "P-384",
            "use": "sig",
            "alg": "ES256",
            "kid": TEST_KID,
            "x": TEST_JWK_X,
            "y": TEST_JWK_Y,
        }),
    );
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidKey
    );
}

#[test]
fn es256_token_against_rsa_family_material_denies() {
    // Cross-family: an RSA JWK selected by `kid` for an ES256 token. No `alg`
    // is declared, so the advisory check is silent; the family mismatch alone
    // must deny.
    let verifier = install_jwk_for(
        vec![Algorithm::ES256],
        json!({
            "kty": "RSA",
            "use": "sig",
            "kid": TEST_KID,
            "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM64",
            "e": "AQAB",
        }),
    );
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidKey
    );
}

#[test]
fn es256_token_against_ed25519_okp_material_denies() {
    // Cross-family the other direction: an OKP/Ed25519 JWK for an ES256 token.
    let verifier = install_jwk_for(
        vec![Algorithm::ES256],
        json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "use": "sig",
            "kid": TEST_KID,
            "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo",
        }),
    );
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidKey
    );
}

#[test]
fn key_material_binding_covers_every_accepted_algorithm() {
    // Exact-shape matrix over `key_material_matches` for every algorithm the
    // policy can accept (`is_asymmetric_algorithm`). Each accepted algorithm
    // must match exactly its required family/curve and reject a representative
    // of every other family/curve, so any single mapping mutation goes red.
    use jsonwebtoken::jwk::{
        AlgorithmParameters, EllipticCurve, EllipticCurveKeyParameters, EllipticCurveKeyType,
        OctetKeyPairParameters, OctetKeyPairType, OctetKeyParameters, OctetKeyType,
        RSAKeyParameters, RSAKeyType,
    };

    // One representative parameter set per distinguishable key material.
    let ec_p256 = AlgorithmParameters::EllipticCurve(EllipticCurveKeyParameters {
        key_type: EllipticCurveKeyType::EC,
        curve: EllipticCurve::P256,
        x: String::new(),
        y: String::new(),
    });
    let ec_p384 = AlgorithmParameters::EllipticCurve(EllipticCurveKeyParameters {
        key_type: EllipticCurveKeyType::EC,
        curve: EllipticCurve::P384,
        x: String::new(),
        y: String::new(),
    });
    let okp_ed25519 = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
        key_type: OctetKeyPairType::OctetKeyPair,
        curve: EllipticCurve::Ed25519,
        x: String::new(),
    });
    let rsa = AlgorithmParameters::RSA(RSAKeyParameters {
        key_type: RSAKeyType::RSA,
        n: String::new(),
        e: String::new(),
    });
    // Materials no accepted algorithm may ever match, so a widening regression
    // to an unrepresented family/curve is caught: another EC curve, an OKP with
    // a non-Ed25519 curve, and a symmetric key.
    let ec_p521 = AlgorithmParameters::EllipticCurve(EllipticCurveKeyParameters {
        key_type: EllipticCurveKeyType::EC,
        curve: EllipticCurve::P521,
        x: String::new(),
        y: String::new(),
    });
    let okp_p256 = AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
        key_type: OctetKeyPairType::OctetKeyPair,
        curve: EllipticCurve::P256,
        x: String::new(),
    });
    let oct = AlgorithmParameters::OctetKey(OctetKeyParameters {
        key_type: OctetKeyType::Octet,
        value: String::new(),
    });
    let all = [
        &ec_p256,
        &ec_p384,
        &okp_ed25519,
        &rsa,
        &ec_p521,
        &okp_p256,
        &oct,
    ];

    // (algorithm, the one material shape it must accept).
    let cases = [
        (Algorithm::ES256, &ec_p256),
        (Algorithm::ES384, &ec_p384),
        (Algorithm::EdDSA, &okp_ed25519),
        (Algorithm::RS256, &rsa),
        (Algorithm::RS384, &rsa),
        (Algorithm::RS512, &rsa),
        (Algorithm::PS256, &rsa),
        (Algorithm::PS384, &rsa),
        (Algorithm::PS512, &rsa),
    ];

    for (alg, expected) in cases {
        assert!(
            is_asymmetric_algorithm(alg),
            "case algorithm {alg:?} must be policy-acceptable"
        );
        for material in all {
            let should_match = std::ptr::eq(material, expected)
                || (matches!(expected, AlgorithmParameters::RSA(_))
                    && matches!(material, AlgorithmParameters::RSA(_)));
            assert_eq!(
                key_material_matches(material, alg),
                should_match,
                "algorithm {alg:?} against material {material:?}"
            );
        }
    }
}

#[test]
fn lowercase_hex_nostr_pubkey_is_accepted() {
    let verifier = verifier_with(access_token_policy());
    let real = nostr::Keys::generate().public_key().to_hex();
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "sub_type": "user", NOSTR_PUBKEY_CLAIM: real }),
    );
    let assertion = verifier.verify(&token).expect("verifies");
    assert!(assertion.asserted_key().is_some());
}

#[test]
fn uppercase_nostr_pubkey_denies() {
    let verifier = verifier_with(access_token_policy());
    let upper = nostr::Keys::generate().public_key().to_hex().to_uppercase();
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "sub_type": "user", NOSTR_PUBKEY_CLAIM: upper }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::ClaimRejected
    );
}

#[test]
fn absent_nostr_pubkey_claim_denies() {
    // `nostr_pubkey` absence must unconditionally reject — NIP-FI v2 dropped
    // the per-issuer `require_attested_key` knob that previously made it
    // optional.  This is a direct falsifiable regression test: removing the
    // `None => Err(VerifierError::ClaimRejected)` arm from
    // `parse_nostr_pubkey_claim` must turn this test red.
    let verifier = verifier_with(access_token_policy());
    let token = mint_no_pubkey(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "sub_type": "user" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::ClaimRejected
    );
}

// ---- Time bounds ----------------------------------------------------------

#[test]
fn expired_assertion_denies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "sub_type": "user", "iat": now() - 1200, "exp": now() - 600 }),
    );
    assert_eq!(verifier.verify(&token).unwrap_err(), VerifierError::Expired);
}

#[test]
fn assertion_beyond_maximum_age_denies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "sub_type": "user", "iat": now() - 4000, "exp": now() + 600 }),
    );
    assert_eq!(verifier.verify(&token).unwrap_err(), VerifierError::Expired);
}

// ---- Fractional NumericDate (P2 #4) --------------------------------------
//
// RFC 7519 permits non-integer `NumericDate` seconds, and real IdPs emit them.
// A finite fractional `iat`/`exp`/`nbf` within bounds must verify; NaN,
// infinity, and absurd magnitudes must deny with `InvalidTimeBounds`.

#[test]
fn fractional_iat_and_exp_within_bounds_verify() {
    let verifier = verifier_with(dedicated_policy(ISSUER));
    let iat = now() as f64 - 0.5;
    let exp = now() as f64 + 600.25;
    let token = mint(
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "sub": "u", "iat": iat, "exp": exp }),
    );
    assert!(verifier.verify(&token).is_ok());
}

#[test]
fn fractional_nbf_within_bounds_verifies() {
    let verifier = verifier_with(dedicated_policy(ISSUER));
    let token = mint(
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "sub": "u", "nbf": now() as f64 - 0.75 }),
    );
    assert!(verifier.verify(&token).is_ok());
}

#[test]
fn non_finite_numeric_date_denies() {
    // JSON cannot encode NaN/Infinity as a number, so a non-finite time claim
    // can only arrive as a string. `exp`/`iat` are required spec claims that
    // `decode` rejects first; the optional `nbf` reaches `parse_numeric_date`,
    // whose `as_f64` rejects the string with `InvalidTimeBounds`.
    let verifier = verifier_with(dedicated_policy(ISSUER));
    let token = mint(
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "sub": "u", "nbf": "Infinity" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidTimeBounds
    );
}

#[test]
fn absurd_magnitude_fractional_date_denies() {
    // A fractional `nbf` beyond the representable `i64`-seconds range denies
    // rather than saturating the cast. (`decode` leaves the optional, non-
    // required `nbf` untouched when it fails its own numeric parse, so this
    // reaches `parse_numeric_date`.)
    let verifier = verifier_with(dedicated_policy(ISSUER));
    let token = mint(
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "sub": "u", "nbf": 1.0e30 }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::InvalidTimeBounds
    );
}

// ---- Multi-issuer selection ----------------------------------------------

#[test]
fn unknown_issuer_denies() {
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "u", "client_id": "a", "iss": "https://evil.example" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::UnknownIssuer
    );
}

#[test]
fn same_subject_distinct_issuers_are_distinct_identities() {
    let issuer_a = "https://a.example";
    let issuer_b = "https://b.example";
    let policy_a = dedicated_policy(issuer_a);
    let policy_b = dedicated_policy(issuer_b);
    assert_ne!(policy_a.id(), policy_b.id());

    let mut registry = IssuerRegistry::new();
    registry.insert(policy_a);
    registry.insert(policy_b);
    // Both issuers share the same test signing key here; the source binds a
    // snapshot to each issuer and the verifier selects by authenticated `iss`.
    let verifier = FederatedAssertionVerifier::new(
        registry,
        StaticIssuerKeySource::new([key_set_for(issuer_a), key_set_for(issuer_b)]),
    );

    let sign = |iss: &str| {
        let claims = json!({ "sub": "shared-sub", "iss": iss });
        mint(Some("nip-fi+jwt"), TEST_KID, claims)
    };
    let a = verifier.verify(&sign(issuer_a)).expect("a verifies");
    let b = verifier.verify(&sign(issuer_b)).expect("b verifies");
    assert_eq!(a.identity().subject(), b.identity().subject());
    assert_ne!(a.identity().issuer(), b.identity().issuer());
    assert_ne!(a.assertion_policy_id(), b.assertion_policy_id());
}

// ---- Cross-issuer key-source confusion (CRITICAL #1) ---------------------

#[test]
fn cross_issuer_token_cannot_mint_through_any_seam() {
    // The structural regression for the key-source-confusion bypass. Two seams
    // are covered:
    //
    // 1. Request seam: issuer B signs a token with its own real key while the
    //    signed claim says `iss=A`. `verify` takes only the token and resolves
    //    the snapshot from the trusted source keyed by the authenticated `iss`,
    //    so B's keys can never authenticate a token claiming issuer A.
    //
    // 2. Authority-construction seam: an external `buzz_auth` consumer cannot
    //    even build the relabelling authority. `AssertionKeySet` has no public
    //    constructor and `IssuerKeySource` is sealed, so external code can
    //    neither put B's JWKS into a snapshot labelled A nor supply its own
    //    source that does. The exploit that minted sealed `(A, victim)` at the
    //    public verifier constructor no longer type-checks — see the two
    //    `compile_fail` doctests on `AssertionKeySet` (`verifier.rs:70-73`) and
    //    `IssuerKeySource` (`verifier.rs:142-148`).
    let issuer_a = "https://a.example";
    let issuer_b = "https://b.example";

    // Each issuer's source snapshot carries only its own real public key. Even
    // here — inside the crate, using the test-only constructor — the snapshot's
    // issuer label is bound to the JWKS it actually authenticates.
    let key_a = key_set_for(issuer_a);
    let key_b = AssertionKeySet::new(
        issuer_b.to_owned(),
        1,
        jwks_with_coords(TEST_KID, TEST_JWK_X_B, TEST_JWK_Y_B),
        future_deadline(),
    )
    .unwrap();

    let mut registry = IssuerRegistry::new();
    registry.insert(dedicated_policy(issuer_a));
    registry.insert(dedicated_policy(issuer_b));
    let verifier =
        FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([key_a, key_b]));

    // Token signed by B's key, claiming `iss=A`. The verifier selects issuer
    // A's policy and issuer A's snapshot; B's signature fails against A's key.
    let forged = mint_signed_by(
        TEST_EC_PKCS8_PEM_B,
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "iss": issuer_a, "sub": "victim" }),
    );
    assert_eq!(
        verifier.verify(&forged).unwrap_err(),
        VerifierError::InvalidSignatureOrClaims,
        "B-signed token claiming iss=A must not mint an A identity"
    );

    // Sanity: each issuer's own honestly-signed token verifies under its bound
    // snapshot, so the deny above is the forgery, not a broken key source.
    let honest_a = mint_signed_by(
        TEST_EC_PKCS8_PEM,
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "iss": issuer_a, "sub": "u" }),
    );
    let honest_b = mint_signed_by(
        TEST_EC_PKCS8_PEM_B,
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "iss": issuer_b, "sub": "u" }),
    );
    assert_eq!(
        verifier.verify(&honest_a).unwrap().identity().issuer(),
        issuer_a
    );
    assert_eq!(
        verifier.verify(&honest_b).unwrap().identity().issuer(),
        issuer_b
    );
}

#[test]
fn registered_issuer_without_key_snapshot_is_unavailable_not_rejected() {
    // A registered issuer whose trusted source has no snapshot is an
    // unreadable authoritative dependency, not rejected evidence: the token
    // may be valid. It maps to AuthorizationUnavailable (503), never
    // EvidenceRejected, so a JWKS gap can't masquerade as a bad token.
    let registry = {
        let mut r = IssuerRegistry::new();
        r.insert(dedicated_policy(ISSUER));
        r
    };
    // Empty key source: the issuer is registered but has no snapshot.
    let verifier = FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([]));
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::KeySourceUnavailable);
    assert_eq!(err.denial_class(), DenialClass::AuthorizationUnavailable);
}

// ---- Dependency-independent checks precede key-source lookup (P1 #2) ------
//
// Malformed evidence must be classified (403) before an unreadable snapshot
// could yield a 503, at the front end of the pipeline (the mirror of round-3's
// offline-before-`CurrentStatus`-deferral at the back end). With an empty key
// source, a wrong-`typ` or structurally malformed token must still deny as
// rejected evidence, never `KeySourceUnavailable` (NIP-FI.md:151-171, :458-475).

fn verifier_with_empty_source() -> FederatedAssertionVerifier<StaticIssuerKeySource> {
    let mut registry = IssuerRegistry::new();
    registry.insert(dedicated_policy(ISSUER));
    FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([]))
}

#[test]
fn wrong_typ_is_rejected_before_key_source_lookup() {
    // A configured issuer whose source has no snapshot: a `typ=JWT` token for a
    // `nip-fi+jwt` policy is rejected evidence (403), not 503.
    let verifier = verifier_with_empty_source();
    let token = mint(Some("JWT"), TEST_KID, json!({ "sub": "u" }));
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::TokenTypeRejected);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
    assert_eq!(err.denial_class().http_status(), 403);
}

#[test]
fn two_segment_garbage_is_rejected_before_key_source_lookup() {
    // Two-segment garbage: the header/claims parsers each read a single fixed
    // segment, so without the explicit structure gate this would reach the
    // outage path. It must deny as malformed evidence (403).
    let verifier = verifier_with_empty_source();
    let header = b64_segment(r#"{"alg":"ES256","kid":"test-key-1","typ":"nip-fi+jwt"}"#);
    let claims = b64_segment(r#"{"iss":"https://issuer.example","sub":"u"}"#);
    let token = format!("{header}.{claims}");
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::MalformedToken);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
    assert_eq!(err.denial_class().http_status(), 403);
}

#[test]
fn four_segment_garbage_is_rejected_before_key_source_lookup() {
    // Four-segment garbage likewise denies as malformed evidence (403), not an
    // outage 503.
    let verifier = verifier_with_empty_source();
    let header = b64_segment(r#"{"alg":"ES256","kid":"test-key-1","typ":"nip-fi+jwt"}"#);
    let claims = b64_segment(r#"{"iss":"https://issuer.example","sub":"u"}"#);
    let token = format!("{header}.{claims}.sig.extra");
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::MalformedToken);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
}

#[test]
fn empty_signature_is_rejected_before_key_source_lookup() {
    // Three segments but an empty signature: a dependency-independent malformed
    // shape (only cryptographic validity needs the key). It must deny as
    // malformed evidence (403), not defer to the outage seam (503).
    let verifier = verifier_with_empty_source();
    let header = b64_segment(r#"{"alg":"ES256","kid":"test-key-1","typ":"nip-fi+jwt"}"#);
    let claims = b64_segment(r#"{"iss":"https://issuer.example","sub":"u"}"#);
    let token = format!("{header}.{claims}.");
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::MalformedToken);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
    assert_eq!(err.denial_class().http_status(), 403);
}

#[test]
fn non_base64url_signature_is_rejected_before_key_source_lookup() {
    // A non-empty but invalid-base64url signature (`!` is not in the alphabet)
    // is also a dependency-independent malformed shape: 403, not 503.
    let verifier = verifier_with_empty_source();
    let header = b64_segment(r#"{"alg":"ES256","kid":"test-key-1","typ":"nip-fi+jwt"}"#);
    let claims = b64_segment(r#"{"iss":"https://issuer.example","sub":"u"}"#);
    let token = format!("{header}.{claims}.!");
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::MalformedToken);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
    assert_eq!(err.denial_class().http_status(), 403);
}

#[test]
fn misbinding_key_source_is_rejected_by_defensive_check() {
    // Defense-in-depth: even the crate-owned source, if it returned a snapshot
    // labelled for a different issuer than requested, must not authenticate.
    // The verifier re-checks the returned snapshot's issuer against the
    // selected policy and denies on mismatch, so a source contract violation
    // cannot cross issuers even though the honest source never triggers this.
    let mut registry = IssuerRegistry::new();
    registry.insert(dedicated_policy(ISSUER));
    let verifier = FederatedAssertionVerifier::new(
        registry,
        StaticIssuerKeySource::misbinding(key_set_for("https://other.example")),
    );
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::IssuerKeyMismatch
    );
}

// ---- Duplicate-member rejection (IMPORTANT #2) ---------------------------
//
// Duplicate members are rejected while parsing the protected header and the
// claims segment — both before signature verification — so these tokens carry
// a dummy signature; the parse denies first.

#[test]
fn duplicate_claim_member_denies() {
    let verifier = verifier_with(access_token_policy());
    // Duplicate `sub`: last-wins parsing would silently pick "attacker".
    let claims = format!(
        r#"{{"iss":"{ISSUER}","aud":"{AUDIENCE}","iat":{iat},"exp":{exp},"client_id":"a","sub_type":"user","sub":"victim","sub":"attacker"}}"#,
        iat = now(),
        exp = now() + 600,
    );
    let header = r#"{"alg":"ES256","kid":"test-key-1","typ":"at+jwt"}"#;
    let token = format!("{}.{}.AAAA", b64_segment(header), b64_segment(&claims));
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::DuplicateMember
    );
}

#[test]
fn duplicate_header_member_denies() {
    let verifier = verifier_with(access_token_policy());
    // Duplicate `alg` in the protected header; last-wins would read "none".
    let header = r#"{"alg":"ES256","alg":"none","kid":"test-key-1","typ":"at+jwt"}"#;
    let claims = format!(
        r#"{{"iss":"{ISSUER}","aud":"{AUDIENCE}","iat":{iat},"exp":{exp},"client_id":"a","sub":"u","sub_type":"user"}}"#,
        iat = now(),
        exp = now() + 600,
    );
    let token = format!("{}.{}.AAAA", b64_segment(header), b64_segment(&claims));
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::DuplicateMember
    );
}

// ---- CurrentStatus deferral (IMPORTANT #7) -------------------------------

fn current_status_policy() -> IssuerPolicy {
    IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::CurrentStatus,
        vec![Algorithm::ES256],
        60,
        3600,
        Some(120), // maximum_status_age required for current-status
        test_jwks_contract(),
    )
    .expect("valid current-status policy")
}

#[test]
fn current_status_policy_denies_without_witness() {
    let verifier = verifier_with(current_status_policy());
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::StatusWitnessUnavailable);
    // An unreadable required current dependency is authorization-unavailable
    // (503), never rejected evidence (403): the token may be perfectly valid.
    assert_eq!(err.denial_class(), DenialClass::AuthorizationUnavailable);
    assert_eq!(err.denial_class().http_status(), 503);
}

// A `current-status` policy must complete every offline check before deferring
// to the (unavailable) status witness. Invalid attacker input therefore denies
// as `evidence_rejected` (403), not `authorization_unavailable` (503): a bad
// token can never masquerade as an availability signal (NIP-FI.md:459-476).

#[test]
fn current_status_invalid_signature_is_evidence_rejected_not_unavailable() {
    let verifier = verifier_with(current_status_policy());
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    // A well-formed but cryptographically wrong signature completes every
    // offline check and denies as rejected evidence before deferral.
    let token = tamper_signature(&token);
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::InvalidSignatureOrClaims);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
    assert_eq!(err.denial_class().http_status(), 403);
}

#[test]
fn current_status_wrong_audience_is_evidence_rejected_not_unavailable() {
    let verifier = verifier_with(current_status_policy());
    let token = mint(
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "sub": "u", "aud": "https://other.example" }),
    );
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::InvalidSignatureOrClaims);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
}

#[test]
fn current_status_malformed_claim_is_evidence_rejected_not_unavailable() {
    // A non-integer `exp` is a malformed time claim, rejected during offline
    // signature/claim validation. Under a current-status policy it must still
    // deny as rejected evidence (403), reached only because offline validation
    // runs before the status deferral.
    let verifier = verifier_with(current_status_policy());
    let token = mint(
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "sub": "u", "exp": "not-a-number" }),
    );
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::InvalidSignatureOrClaims);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
}

#[test]
fn current_status_expired_token_is_evidence_rejected_not_unavailable() {
    // Time validation precedes the status deferral, so an expired current-status
    // token is rejected evidence (403), not authorization-unavailable (503).
    let verifier = verifier_with(current_status_policy());
    let token = mint(
        Some("nip-fi+jwt"),
        TEST_KID,
        json!({ "sub": "u", "iat": now() - 1200, "exp": now() - 600 }),
    );
    let err = verifier.verify(&token).unwrap_err();
    assert_eq!(err, VerifierError::Expired);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
}

// ---- Authenticated key-set bound (P1 #1) ---------------------------------

fn jwks_with_n_keys(n: usize) -> JwkSet {
    let keys: Vec<Value> = (0..n)
        .map(|i| {
            json!({
                "kty": "EC",
                "crv": "P-256",
                "use": "sig",
                "alg": "ES256",
                "kid": format!("k{i}"),
                "x": TEST_JWK_X,
                "y": TEST_JWK_Y,
            })
        })
        .collect();
    serde_json::from_value(json!({ "keys": keys })).expect("valid JWKS")
}

#[test]
fn oversized_key_snapshot_cannot_be_installed() {
    // The authenticated key set is bounded before lookup (NIP-FI.md:166-171):
    // `verify` scans it by an attacker-controlled `kid`, so a snapshot beyond
    // MAX_JWKS_KEYS cannot even be constructed — the O(keys) scan is capped at
    // the source. An attacker-installed 100k-key JWKS is impossible.
    let oversized = jwks_with_n_keys(MAX_JWKS_KEYS + 1);
    assert!(
        AssertionKeySet::new(ISSUER.to_owned(), 1, oversized, future_deadline()).is_none(),
        "a snapshot exceeding MAX_JWKS_KEYS must be rejected at construction"
    );
    // The bound itself is admissible.
    let at_bound = jwks_with_n_keys(MAX_JWKS_KEYS);
    assert!(
        AssertionKeySet::new(ISSUER.to_owned(), 1, at_bound, future_deadline()).is_some(),
        "a snapshot at exactly MAX_JWKS_KEYS is accepted"
    );
}

#[test]
fn empty_key_snapshot_cannot_be_installed() {
    let empty: JwkSet = serde_json::from_value(json!({ "keys": [] })).expect("valid JWKS");
    assert!(AssertionKeySet::new(ISSUER.to_owned(), 1, empty, future_deadline()).is_none());
}

// ---- Fixed `sub` identity coordinate (P1 #2) -----------------------------

#[test]
fn identity_subject_is_the_jwt_sub_claim() {
    // Identity is exactly `(iss, sub)`; the subject coordinate is the JWT `sub`
    // claim, hard-coded and never configurable (NIP-FI.md:35-41, :173-175).
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": "user-123", "email": "mutable@example.com", "client_id": "app-1", "sub_type": "user" }),
    );
    let assertion = verifier.verify(&token).expect("verifies");
    // The sealed subject is `sub`, never a mutable attribute like `email`.
    assert_eq!(assertion.identity().subject(), "user-123");
    assert_ne!(assertion.identity().subject(), "mutable@example.com");
}

#[test]
fn token_without_sub_denies_even_with_other_identifier_claims() {
    // With `sub` absent, no other claim (email, employee number, …) can stand
    // in as the identity coordinate: the token denies as rejected evidence.
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "email": "mutable@example.com", "client_id": "app-1", "sub_type": "user" }),
    );
    assert_eq!(
        verifier.verify(&token).unwrap_err(),
        VerifierError::ClaimRejected
    );
}

// ---- Revalidation dependencies: confidential JWS + key deadline (P1 #4) --

#[test]
fn revalidation_dependencies_carry_key_deadline_and_confidential_assertion() {
    // The sealed result carries the key-snapshot hard deadline and a
    // confidential handle to the exact compact JWS, so final admission can
    // revalidate the byte-identical assertion under current state
    // (NIP-FI.md:240-249, :371-395).
    let deadline = future_deadline();
    let key_set = AssertionKeySet::new(ISSUER.to_owned(), 7, test_jwks(TEST_KID), deadline)
        .expect("valid key set");
    let mut registry = IssuerRegistry::new();
    registry.insert(dedicated_policy(ISSUER));
    let verifier = FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([key_set]));
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    let assertion = verifier.verify(&token).expect("verifies");
    let deps = assertion.revalidation_dependencies();
    assert_eq!(deps.verification_key_id(), TEST_KID);
    assert_eq!(deps.key_snapshot_generation(), 7);
    assert_eq!(deps.key_snapshot_hard_deadline(), deadline);
    // The confidential handle is the exact compact JWS, byte-for-byte.
    assert_eq!(deps.confidential_assertion().compact_jws(), token);
    // The key-snapshot deadline is a bounds-class member of authority_deadlines.
    assert!(assertion.authority_deadlines().contains(&deadline));
}

/// A verifier for `ISSUER` serving a dedicated-assertion policy over one
/// key snapshot at an explicit generation and JWKS — the changed-snapshot
/// dimension the JWKS-ADD/REMOVE contracts turn on.
fn dedicated_verifier_at(
    generation: u64,
    jwks: JwkSet,
) -> FederatedAssertionVerifier<StaticIssuerKeySource> {
    let key_set = AssertionKeySet::new(ISSUER.to_owned(), generation, jwks, future_deadline())
        .expect("valid key set");
    let mut registry = IssuerRegistry::new();
    registry.insert(dedicated_policy(ISSUER));
    FederatedAssertionVerifier::new(registry, StaticIssuerKeySource::new([key_set]))
}

#[test]
fn retained_key_revalidates_under_changed_snapshot_and_replacement_denies() {
    // FI-TRACE-JWKS-ADD / FI-TRACE-JWKS-REMOVE at the verifier seam. Both
    // contracts turn on a *changed authenticated generation*, not on source
    // outage (covered separately by
    // `registered_issuer_without_key_snapshot_is_unavailable_not_rejected`).
    // Mint once at generation 1, then revalidate the exact carried JWS against
    // two distinct generation-2 snapshots.
    let token = mint(Some("nip-fi+jwt"), TEST_KID, json!({ "sub": "u" }));
    let first = dedicated_verifier_at(1, test_jwks(TEST_KID))
        .verify(&token)
        .expect("verifies at generation 1");
    assert_eq!(
        first.revalidation_dependencies().key_snapshot_generation(),
        1
    );
    let carried = first
        .revalidation_dependencies()
        .confidential_assertion()
        .compact_jws()
        .to_owned();

    // JWKS-ADD: a later generation that *retains* the signing key revalidates
    // the byte-identical assertion, now bound to the new generation.
    let revalidated = dedicated_verifier_at(2, test_jwks(TEST_KID))
        .verify(&carried)
        .expect("retained key revalidates under the changed snapshot");
    assert_eq!(first.identity().subject(), revalidated.identity().subject());
    assert_eq!(
        revalidated
            .revalidation_dependencies()
            .key_snapshot_generation(),
        2
    );

    // JWKS-REMOVE: a still-readable later generation containing *only a
    // replacement key* (the original `kid` is gone) denies the same evidence
    // as rejected — no `kid` match, never sealed under a substituted key. This
    // is a changed snapshot, not a source outage.
    let err = dedicated_verifier_at(
        2,
        jwks_with_coords("replacement-key", TEST_JWK_X_B, TEST_JWK_Y_B),
    )
    .verify(&carried)
    .unwrap_err();
    assert_eq!(err, VerifierError::AmbiguousKeyId);
    assert_eq!(err.denial_class(), DenialClass::EvidenceRejected);
}

#[test]
fn subject_bytes_are_preserved_exactly_not_trimmed() {
    // A subject with surrounding whitespace must survive verbatim: trimming
    // would collapse distinct byte strings into one identity.
    let verifier = verifier_with(access_token_policy());
    let token = mint(
        Some("at+jwt"),
        TEST_KID,
        json!({ "sub": " user-123 ", "client_id": "app-1", "sub_type": "user" }),
    );
    let assertion = verifier.verify(&token).expect("verifies");
    assert_eq!(assertion.identity().subject(), " user-123 ");
}

// ---- Deterministic contract IDs ------------------------------------------

#[test]
fn assertion_policy_id_is_deterministic_and_semantic() {
    let p1 = access_token_policy();
    let p2 = access_token_policy();
    assert_eq!(p1.id(), p2.id(), "same contract => same id");

    let changed = access_token_policy_with(subject_class_reject());
    let changed = IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        changed.token_class().clone(),
        FreshnessClass::OfflineJwt,
        vec![Algorithm::ES256],
        120, // different skew => different semantics
        3600,
        None,
        test_jwks_contract(),
    )
    .unwrap();
    assert_ne!(p1.id(), changed.id());
}

// ---- `maximum_status_age` applicability (P1 #3) --------------------------
//
// `maximum_status_age` is read only under `current-status`. An `offline-jwt`
// policy that accepted it would hash it into the ID, so two semantically
// identical offline policies (`None` vs `Some(120)`) would derive different
// IDs. It is rejected at construction, keeping the canonical encoding total
// over valid configs (NIP-FI.md:219-237).

#[test]
fn offline_policy_rejects_inapplicable_maximum_status_age() {
    let err = IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        vec![Algorithm::ES256],
        60,
        3600,
        Some(120),
        test_jwks_contract(),
    )
    .unwrap_err();
    assert_eq!(err, IssuerPolicyError::InapplicableMaximumStatusAge);
}

#[test]
fn offline_policy_accepts_absent_maximum_status_age() {
    // The only valid offline shape: `None`. Construction succeeds.
    assert!(IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        vec![Algorithm::ES256],
        60,
        3600,
        None,
        test_jwks_contract(),
    )
    .is_ok());
}

#[test]
fn current_status_policy_still_requires_positive_maximum_status_age() {
    // The applicability rule must not weaken the existing current-status
    // requirement: `None` and `Some(0)` both deny.
    let missing = IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::CurrentStatus,
        vec![Algorithm::ES256],
        60,
        3600,
        None,
        test_jwks_contract(),
    )
    .unwrap_err();
    assert_eq!(missing, IssuerPolicyError::MissingMaximumStatusAge);
    let zero = IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::CurrentStatus,
        vec![Algorithm::ES256],
        60,
        3600,
        Some(0),
        test_jwks_contract(),
    )
    .unwrap_err();
    assert_eq!(zero, IssuerPolicyError::InvalidTimeBounds);
}

#[test]
fn assertion_policy_id_moves_with_subject_class_contract() {
    // The subject-class contract is a normative input to the policy ID.
    let base = access_token_policy_with(subject_class_reject());
    let different_values = access_token_policy_with(
        SubjectClassContract::new(
            "sub_type".to_owned(),
            vec!["human".to_owned()], // different resource-owner value set
            vec!["client".to_owned()],
            ClientSubjectPosture::Reject,
        )
        .unwrap(),
    );
    let different_posture = access_token_policy_with(
        SubjectClassContract::new(
            "sub_type".to_owned(),
            vec!["user".to_owned()],
            vec!["client".to_owned()],
            ClientSubjectPosture::AcceptNonColliding, // different posture
        )
        .unwrap(),
    );
    assert_ne!(base.id(), different_values.id());
    assert_ne!(base.id(), different_posture.id());
}

#[test]
fn assertion_policy_id_is_invariant_under_audience_permutation_and_duplicates() {
    // Audiences are consumed as a membership set, so caller order and
    // duplicates carry no semantics and must not move the policy ID.
    let base = dedicated_policy_with_audiences(vec![
        "https://a.example".to_owned(),
        "https://b.example".to_owned(),
    ]);
    let permuted = dedicated_policy_with_audiences(vec![
        "https://b.example".to_owned(),
        "https://a.example".to_owned(),
    ]);
    let duplicated = dedicated_policy_with_audiences(vec![
        "https://b.example".to_owned(),
        "https://a.example".to_owned(),
        "https://a.example".to_owned(),
    ]);
    assert_eq!(base.id(), permuted.id());
    assert_eq!(base.id(), duplicated.id());
    // A different audience set still moves the ID.
    let different = dedicated_policy_with_audiences(vec!["https://a.example".to_owned()]);
    assert_ne!(base.id(), different.id());
}

#[test]
fn assertion_policy_id_is_invariant_under_algorithm_permutation_and_duplicates() {
    let base = dedicated_policy_with_algorithms(vec![Algorithm::ES256, Algorithm::RS256]);
    let permuted = dedicated_policy_with_algorithms(vec![Algorithm::RS256, Algorithm::ES256]);
    let duplicated = dedicated_policy_with_algorithms(vec![
        Algorithm::RS256,
        Algorithm::ES256,
        Algorithm::RS256,
    ]);
    assert_eq!(base.id(), permuted.id());
    assert_eq!(base.id(), duplicated.id());
    let different = dedicated_policy_with_algorithms(vec![Algorithm::ES256]);
    assert_ne!(base.id(), different.id());
}

#[test]
fn assertion_policy_id_is_invariant_under_subject_class_value_permutation_and_duplicates() {
    let base = access_token_policy_with(
        SubjectClassContract::new(
            "sub_type".to_owned(),
            vec!["user".to_owned(), "owner".to_owned()],
            vec!["client".to_owned()],
            ClientSubjectPosture::Reject,
        )
        .unwrap(),
    );
    let permuted = access_token_policy_with(
        SubjectClassContract::new(
            "sub_type".to_owned(),
            vec!["owner".to_owned(), "user".to_owned(), "user".to_owned()],
            vec!["client".to_owned()],
            ClientSubjectPosture::Reject,
        )
        .unwrap(),
    );
    assert_eq!(base.id(), permuted.id());
}

// ---- JwksSourceContract in AssertionPolicyId ------------------------------
//
// Per the NIP-FI spec ("Policy identity and snapshots"): `assertion_policy_id`
// covers "authenticated key/status-source contracts" and "time rules". The
// three contract fields are immutable contract identity, not mutable state —
// changing any one of them changes which keys the runtime trusts or how long
// it trusts them, invalidating all prepared evidence against the old contract.
// Key rotation (JWKS content change) leaves all three unchanged and must NOT
// move the ID.

/// Helper: build a policy with the given `JwksSourceContract`.
fn policy_with_contract(contract: crate::nip_fi::jwks::JwksSourceContract) -> IssuerPolicy {
    IssuerPolicy::new(
        ISSUER.to_owned(),
        vec![AUDIENCE.to_owned()],
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        vec![Algorithm::ES256],
        60,
        3600,
        None,
        contract,
    )
    .expect("valid policy")
}

#[test]
fn assertion_policy_id_moves_when_jwks_uri_changes() {
    // The JWKS URI selects the authenticated key source. A different URI may
    // serve different keys — the policy ID must change.
    //
    // Mutation (omit URI from hash): both policies hash identically despite
    // different endpoints; this test turns red.
    let base = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", ISSUER),
            300,
            3600,
        )
        .unwrap(),
    );
    let different_uri = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks-alt.json", ISSUER),
            300,
            3600,
        )
        .unwrap(),
    );
    assert_ne!(
        base.id(),
        different_uri.id(),
        "JWKS URI change must move assertion_policy_id"
    );
}

#[test]
fn assertion_policy_id_moves_when_refresh_interval_changes() {
    // The refresh interval defines bounded refresh behavior. A longer interval
    // allows stale keys to persist longer — the policy ID must change.
    //
    // Mutation (omit refresh_interval from hash): both policies hash
    // identically; this test turns red.
    let base = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", ISSUER),
            300,
            3600,
        )
        .unwrap(),
    );
    let different_interval = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", ISSUER),
            600, // doubled
            3600,
        )
        .unwrap(),
    );
    assert_ne!(
        base.id(),
        different_interval.id(),
        "refresh_interval_seconds change must move assertion_policy_id"
    );
}

#[test]
fn assertion_policy_id_moves_when_hard_deadline_changes() {
    // The hard deadline defines the source's accepted time rule; every
    // per-snapshot deadline the verifier seals into `VerifiedAssertion`
    // derives from this. A looser deadline extends the valid window beyond
    // what the new policy intends — the policy ID must change.
    //
    // Mutation (omit key_snapshot_hard_deadline from hash): both policies
    // hash identically; this test turns red.
    let base = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", ISSUER),
            300,
            3600,
        )
        .unwrap(),
    );
    let different_deadline = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", ISSUER),
            300,
            7200, // doubled
        )
        .unwrap(),
    );
    assert_ne!(
        base.id(),
        different_deadline.id(),
        "key_snapshot_hard_deadline_seconds change must move assertion_policy_id"
    );
}

#[test]
fn assertion_policy_id_is_stable_for_same_jwks_contract() {
    // URI canonicalization is deterministic: the same validated URI, interval,
    // and deadline always hash to the same policy ID regardless of call order.
    let c1 = crate::nip_fi::jwks::JwksSourceContract::new(
        format!("{}/.well-known/jwks.json", ISSUER),
        300,
        3600,
    )
    .unwrap();
    let c2 = crate::nip_fi::jwks::JwksSourceContract::new(
        format!("{}/.well-known/jwks.json", ISSUER),
        300,
        3600,
    )
    .unwrap();
    let p1 = policy_with_contract(c1);
    let p2 = policy_with_contract(c2);
    assert_eq!(
        p1.id(),
        p2.id(),
        "same JWKS contract must produce identical assertion_policy_id"
    );
}

#[test]
fn identical_contract_produces_stable_assertion_policy_id() {
    // `AssertionPolicyId` is derived from the contract fields only — not from
    // JWKS key material. This means JWKS key additions/removals (runtime
    // rotation) cannot change the policy ID; only changes to the contract
    // itself (JWKS URI, refresh interval, hard deadline) would do so.
    //
    // This test verifies the structural invariant: two `IssuerPolicy` values
    // built from identical contracts produce the same `AssertionPolicyId`,
    // regardless of when or how many times the ID is derived. Because key
    // material never flows into `derive_assertion_policy_id`, the ID is
    // stable for the lifetime of a given contract.
    let p1 = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", ISSUER),
            300,
            3600,
        )
        .unwrap(),
    );
    let p2 = policy_with_contract(
        crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", ISSUER),
            300,
            3600,
        )
        .unwrap(),
    );
    // Identical contract → identical ID: key material is not part of the hash.
    assert_eq!(
        p1.id(),
        p2.id(),
        "identical contract must produce the same assertion_policy_id (key material is not hashed)"
    );
}

#[test]
fn scope_capture_is_canonical_under_order_and_duplicates() {
    // The `scope` claim is a space-delimited set: equivalent scope sets must
    // seal byte-equal capabilities regardless of token order or repetition.
    let verifier = verifier_with(dedicated_policy(ISSUER));
    let a = verifier
        .verify(&mint(
            Some("nip-fi+jwt"),
            TEST_KID,
            json!({ "sub": "u", "scope": "read write admin" }),
        ))
        .expect("verifies");
    let b = verifier
        .verify(&mint(
            Some("nip-fi+jwt"),
            TEST_KID,
            json!({ "sub": "u", "scope": "admin write read write" }),
        ))
        .expect("verifies");
    assert_eq!(a.capabilities().entries(), b.capabilities().entries());
    assert_eq!(
        a.capabilities().entries(),
        &[
            ("scope".to_owned(), "admin".to_owned()),
            ("scope".to_owned(), "read".to_owned()),
            ("scope".to_owned(), "write".to_owned()),
        ]
    );
}

#[test]
fn transport_contract_id_is_stable() {
    assert_eq!(
        TransportContractId::core_client_attached(),
        TransportContractId::core_client_attached()
    );
    assert_eq!(CLIENT_ATTACHED_HEADER, "Nostr-Federated-Identity");
}

// ---- Exact-wire-text denial contract (all four classes) ------------------

#[test]
fn denial_classes_carry_exact_wire_text() {
    let m = DenialClass::MissingEvidence;
    assert_eq!(m.nostr_text(), "auth-required: authentication required");
    assert_eq!(m.http_status(), 401);
    assert_eq!(m.http_body(), "authentication required\n");
    assert_eq!(m.www_authenticate(), Some("Nostr"));
    assert_eq!(m.content_type(), "text/plain; charset=utf-8");

    let e = DenialClass::EvidenceRejected;
    assert_eq!(e.nostr_text(), "restricted: evidence rejected");
    assert_eq!(e.http_status(), 403);
    assert_eq!(e.http_body(), "evidence rejected\n");
    assert_eq!(e.www_authenticate(), None);

    let d = DenialClass::AuthorizationDenied;
    assert_eq!(d.nostr_text(), "restricted: authorization denied");
    assert_eq!(d.http_status(), 403);
    assert_eq!(d.http_body(), "authorization denied\n");

    let u = DenialClass::AuthorizationUnavailable;
    assert_eq!(u.nostr_text(), "restricted: authorization unavailable");
    assert_eq!(u.http_status(), 503);
    assert_eq!(u.http_body(), "authorization unavailable\n");
}
