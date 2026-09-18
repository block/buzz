//! Startup validation for the NIP-FI assertion runtime.
//!
//! [`validate_nip_fi_config`] is the production entry point. It rejects any
//! configuration that would make the runtime unsafe, incomplete, or ambiguous
//! before the relay accepts any protected traffic. The relay MUST call this and
//! refuse to start on error in [`Enforce`][NipFiMode::Enforce] mode
//! (`FI-INV-14`, `FI-INV-15`).

use super::config::{FreshnessClass, IssuerRegistry};
use super::jwks::IssuerJwksConfig;

/// Variant names are stable contract values; do not rename without a
/// `VERIFIER_CONTRACT_VERSION` bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NipFiMode {
    /// NIP-FI is disabled. Protected ingresses are unreachable or absent.
    Off,
    /// Production enforcement: every protected ingress requires valid
    /// federated assertion evidence. The relay MUST call
    /// [`validate_nip_fi_config`] before accepting traffic in this mode.
    Enforce,
    /// All protected routes deny unconditionally. Used when a prior
    /// enforce-mode deployment was misconfigured and must fail closed while
    /// the operator repairs configuration. [FI-INV-14]
    DenyProtected,
}

/// Every variant corresponds to a concrete, operator-actionable defect.
/// No key material, token bytes, or raw claim values appear.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NipFiStartupError {
    /// Registry has no entries; enforce mode requires at least one issuer.
    #[error("NIP-FI enforce mode requires at least one issuer policy")]
    EmptyRegistry,

    /// The duplicate `iss` is omitted to avoid leaking configuration into
    /// operational logs.
    #[error("NIP-FI issuer registry contains a duplicate issuer")]
    DuplicateIssuer,

    /// Every registered issuer requires a JWKS endpoint in enforce mode.
    #[error("NIP-FI issuer has no JWKS configuration")]
    MissingJwksConfig,

    /// Mismatched configs are rejected to prevent silent key-source confusion.
    #[error("NIP-FI JWKS config issuer does not match any registered policy")]
    UnmatchedJwksConfig,

    /// The `JwksSourceContract` embedded in the `IssuerJwksConfig` does not
    /// match the contract in the corresponding `IssuerPolicy`. Both must carry
    /// exactly the same contract to keep a single source of truth per issuer.
    #[error("NIP-FI JWKS config contract does not match the registered policy contract")]
    JwksContractMismatch,

    /// `current-status` requires an authenticated status witness that is not
    /// yet implemented. Use `FreshnessClass::OfflineJwt` instead.
    #[error(
        "NIP-FI current-status freshness is not yet supported; \
         use offline-jwt posture"
    )]
    UnsupportedPosture,
}

/// Validates the complete NIP-FI runtime configuration. On error the relay
/// MUST refuse to start or fall back to [`NipFiMode::DenyProtected`].
pub fn validate_nip_fi_config(
    mode: NipFiMode,
    registry: &IssuerRegistry,
    jwks_configs: &[IssuerJwksConfig],
) -> Result<(), NipFiStartupError> {
    if let NipFiMode::Off | NipFiMode::DenyProtected = mode {
        return Ok(());
    }

    if registry.is_empty() {
        return Err(NipFiStartupError::EmptyRegistry);
    }

    // IssuerRegistry overwrites duplicates silently; assert uniqueness here so
    // a misconfigured multi-issuer call-site is caught before traffic is served.
    {
        let mut seen = std::collections::HashSet::new();
        for policy in registry.all_policies() {
            if !seen.insert(policy.issuer()) {
                return Err(NipFiStartupError::DuplicateIssuer);
            }
        }
    }

    // Reject current-status policies: the status witness is not yet
    // implemented. Fail closed rather than advertise a freshness guarantee the
    // verifier cannot satisfy.
    for policy in registry.all_policies() {
        if policy.freshness() == FreshnessClass::CurrentStatus {
            return Err(NipFiStartupError::UnsupportedPosture);
        }
    }

    // Build JWKS map, rejecting duplicates. Two configs for the same issuer
    // would make the effective endpoint selection order-dependent.
    let mut jwks_map: std::collections::HashMap<&str, &IssuerJwksConfig> =
        std::collections::HashMap::with_capacity(jwks_configs.len());
    for config in jwks_configs {
        if jwks_map.insert(config.issuer.as_str(), config).is_some() {
            return Err(NipFiStartupError::DuplicateIssuer);
        }
    }

    for config in jwks_configs {
        if registry.policy_for_issuer(&config.issuer).is_none() {
            return Err(NipFiStartupError::UnmatchedJwksConfig);
        }
        // Contract fields are pre-validated inside `JwksSourceContract::new`
        // at `IssuerPolicy` construction. Enforce that the config carries the
        // same contract as the policy — a mismatch would mean two independent
        // copies of the URI/timing drifted apart, violating the single-source-
        // of-truth invariant.
        let policy = registry.policy_for_issuer(&config.issuer).unwrap();
        if &config.contract != policy.jwks_source_contract() {
            return Err(NipFiStartupError::JwksContractMismatch);
        }
    }

    for policy in registry.all_policies() {
        if !jwks_map.contains_key(policy.issuer()) {
            return Err(NipFiStartupError::MissingJwksConfig);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
