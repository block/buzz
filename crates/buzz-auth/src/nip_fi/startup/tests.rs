use super::*;
use crate::nip_fi::config::{FreshnessClass, IssuerPolicy, IssuerRegistry, TokenClass};
use crate::nip_fi::jwks::{IssuerJwksConfig, JwksSourceContract};
use jsonwebtoken::Algorithm as JwtAlgorithm;

fn test_contract(issuer: &str) -> JwksSourceContract {
    // Build a canonical JWKS URI from the issuer URL. The issuer may already
    // be a full HTTPS URL (e.g. "https://id.example") or a bare hostname.
    let uri = if issuer.starts_with("https://") {
        format!("{}/.well-known/jwks.json", issuer.trim_end_matches('/'))
    } else {
        format!("https://{}/.well-known/jwks.json", issuer)
    };
    JwksSourceContract::new(uri, 300, 3600).expect("valid test contract")
}

fn make_offline_policy(issuer: &str) -> IssuerPolicy {
    IssuerPolicy::new(
        issuer.to_owned(),
        vec![format!("https://relay.example/api")],
        TokenClass::DedicatedNipFi,
        FreshnessClass::OfflineJwt,
        vec![JwtAlgorithm::ES256],
        0,
        3600,
        None,
        test_contract(issuer),
    )
    .unwrap()
}

fn make_status_policy(issuer: &str) -> IssuerPolicy {
    IssuerPolicy::new(
        issuer.to_owned(),
        vec![format!("https://relay.example/api")],
        TokenClass::DedicatedNipFi,
        FreshnessClass::CurrentStatus,
        vec![JwtAlgorithm::ES256],
        0,
        3600,
        Some(60),
        test_contract(issuer),
    )
    .unwrap()
}

fn make_jwks_config(issuer: &str) -> IssuerJwksConfig {
    IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: test_contract(issuer),
    }
}

#[test]
fn off_mode_accepts_empty_registry() {
    let registry = IssuerRegistry::new();
    assert!(validate_nip_fi_config(NipFiMode::Off, &registry, &[]).is_ok());
}

#[test]
fn deny_protected_mode_accepts_empty_registry() {
    let registry = IssuerRegistry::new();
    assert!(validate_nip_fi_config(NipFiMode::DenyProtected, &registry, &[]).is_ok());
}

#[test]
fn enforce_valid_config_passes() {
    let issuer = "https://id.example";
    let mut registry = IssuerRegistry::new();
    registry.insert(make_offline_policy(issuer));

    assert!(
        validate_nip_fi_config(NipFiMode::Enforce, &registry, &[make_jwks_config(issuer)]).is_ok()
    );
}

#[test]
fn enforce_multiple_issuers_passes() {
    let issuers = [
        "https://a.example",
        "https://b.example",
        "https://c.example",
    ];
    let mut registry = IssuerRegistry::new();
    for iss in &issuers {
        registry.insert(make_offline_policy(iss));
    }
    let jwks: Vec<_> = issuers.iter().map(|i| make_jwks_config(i)).collect();
    assert!(validate_nip_fi_config(NipFiMode::Enforce, &registry, &jwks).is_ok());
}

#[test]
fn enforce_empty_registry_rejects() {
    let registry = IssuerRegistry::new();
    let err = validate_nip_fi_config(NipFiMode::Enforce, &registry, &[]).unwrap_err();
    assert_eq!(err, NipFiStartupError::EmptyRegistry);
}

#[test]
fn enforce_issuer_without_jwks_rejects() {
    let issuer = "https://id.example";
    let mut registry = IssuerRegistry::new();
    registry.insert(make_offline_policy(issuer));

    let err = validate_nip_fi_config(NipFiMode::Enforce, &registry, &[]).unwrap_err();
    assert_eq!(err, NipFiStartupError::MissingJwksConfig);
}

#[test]
fn enforce_unmatched_jwks_config_rejects() {
    let issuer = "https://id.example";
    let mut registry = IssuerRegistry::new();
    registry.insert(make_offline_policy(issuer));

    let err = validate_nip_fi_config(
        NipFiMode::Enforce,
        &registry,
        &[make_jwks_config("https://other.example")],
    )
    .unwrap_err();
    assert_eq!(err, NipFiStartupError::UnmatchedJwksConfig);
}

/// A JWKS config whose contract differs from the policy contract must be
/// rejected — a mismatch means two independent copies of URI/timing have
/// drifted, violating the single-source-of-truth invariant.
#[test]
fn enforce_jwks_contract_mismatch_rejects() {
    let issuer = "https://id.example";
    let mut registry = IssuerRegistry::new();
    registry.insert(make_offline_policy(issuer));

    // Config carries a different refresh interval than the policy (300 vs 600).
    let mismatched_config = IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: JwksSourceContract::new(
            format!("{}/.well-known/jwks.json", issuer.trim_end_matches('/')),
            600, // differs from policy contract (300)
            3600,
        )
        .unwrap(),
    };
    assert_eq!(
        validate_nip_fi_config(NipFiMode::Enforce, &registry, &[mismatched_config]).unwrap_err(),
        NipFiStartupError::JwksContractMismatch
    );
}

/// Rejected regardless of whether a JWKS config is present — the verifier
/// has no status witness to satisfy the freshness guarantee.
#[test]
fn enforce_current_status_policy_always_rejects() {
    let issuer = "https://id.example";
    let mut registry = IssuerRegistry::new();
    registry.insert(make_status_policy(issuer));

    assert_eq!(
        validate_nip_fi_config(NipFiMode::Enforce, &registry, &[]).unwrap_err(),
        NipFiStartupError::UnsupportedPosture
    );
    assert_eq!(
        validate_nip_fi_config(NipFiMode::Enforce, &registry, &[make_jwks_config(issuer)])
            .unwrap_err(),
        NipFiStartupError::UnsupportedPosture
    );
}

/// Duplicate JWKS configs for the same issuer must not silently succeed.
#[test]
fn enforce_duplicate_jwks_issuer_in_configs_rejects() {
    let issuer = "https://id.example";
    let mut registry = IssuerRegistry::new();
    registry.insert(make_offline_policy(issuer));

    let jwks = vec![make_jwks_config(issuer), make_jwks_config(issuer)];
    assert!(
        validate_nip_fi_config(NipFiMode::Enforce, &registry, &jwks).is_err(),
        "duplicate JWKS configs must not pass"
    );
}
