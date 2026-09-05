//! NIP-FI relay-level configuration: issuer set, JWKS warm/refresh, and
//! S4 admin-command fields.
//!
//! Parses `BUZZ_NIP_FI_MODE` and `BUZZ_NIP_FI_ISSUERS` from the environment.
//! S3 fields (assertion-level per-issuer config) build the `IssuerRegistry` +
//! `IssuerJwksConfig` slice for `ProductionJwksSource`.  S4 fields
//! (`maximum_command_age_seconds`, `authorized_principals`,
//! `deny_set_capacity`) are parsed alongside so a single JSON array drives both
//! tiers.
//!
//! # Environment variables
//!
//! | Variable | Required | Description |
//! |---|---|---|
//! | `BUZZ_NIP_FI_MODE` | No | `off` (default), `enforce`, or `deny_protected`. |
//! | `BUZZ_NIP_FI_ISSUERS` | If enforce | JSON array of issuer configs. |
//! | `BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS` | If enforce | Per-partition session-lifetime bound. |
//!
//! Missing or empty `BUZZ_NIP_FI_MODE` defaults to `off`.

use buzz_auth::{
    validate_nip_fi_config, FreshnessClass, IssuerJwksConfig, IssuerPolicy, IssuerPolicyError,
    IssuerRegistry, JwksSourceContract, JwtAlgorithm, NipFiMode, NipFiStartupError, TokenClass,
};

use crate::config::ConfigError;

/// Maximum accepted `max_connection_lifetime` in seconds (30 days).
const MAX_CONNECTION_LIFETIME_SECS: u64 = 30 * 24 * 3600;

// ── Per-issuer JSON config ────────────────────────────────────────────────────

/// One entry in the `BUZZ_NIP_FI_ISSUERS` JSON array.
///
/// Contains both assertion-level fields (S3) and optional command-API fields
/// (S4).  All S4 fields default to absent.
///
/// **Minimal S4 example** (assertion enforcement + command API):
/// ```json
/// [
///   {
///     "issuer": "https://idp.example.com",
///     "audiences": ["https://relay.example.com"],
///     "token_class": "nip-fi+jwt",
///     "algorithms": ["ES256"],
///     "skew_seconds": 30,
///     "maximum_assertion_age_seconds": 3600,
///     "jwks_uri": "https://idp.example.com/.well-known/jwks.json",
///     "jwks_refresh_interval_seconds": 300,
///     "jwks_hard_deadline_seconds": 86400,
///     "maximum_command_age_seconds": 30,
///     "authorized_principals": ["admin@idp.example.com"],
///     "deny_set_capacity": 50000
///   }
/// ]
/// ```
#[derive(Debug, serde::Deserialize)]
pub(super) struct IssuerEnvConfig {
    // ── S3 assertion fields ───────────────────────────────────────────────
    /// Exact `iss` value.
    pub issuer: String,
    /// One or more accepted `aud` values.
    pub audiences: Vec<String>,
    /// `"nip-fi+jwt"` (only supported class in this version).
    pub token_class: TokenClassEnvConfig,
    /// Algorithm names, e.g. `["ES256"]`.
    pub algorithms: Vec<String>,
    /// Accepted clock skew in seconds (≤ 300).
    #[serde(default)]
    pub skew_seconds: u64,
    /// `iat + maximum_assertion_age` residual bound in seconds.
    pub maximum_assertion_age_seconds: u64,
    /// HTTPS endpoint serving the JWK Set for this issuer.
    pub jwks_uri: String,
    /// Seconds between JWKS refreshes.
    pub jwks_refresh_interval_seconds: u64,
    /// Hard deadline for accepting a JWKS snapshot in seconds.
    pub jwks_hard_deadline_seconds: u64,

    // ── S4 command-API fields (all optional) ──────────────────────────────
    /// Maximum command JWT age in seconds; `0 < x ≤ 60`.  Required to enable
    /// the disconnect API for this issuer.
    #[serde(default)]
    pub maximum_command_age_seconds: Option<u64>,
    /// Non-empty list of authorized `sub` values.  Required when
    /// `maximum_command_age_seconds` is set.
    #[serde(default)]
    pub authorized_principals: Option<Vec<String>>,
    /// Hard ceiling on live deny entries for this issuer.  Defaults to
    /// [`crate::api::nip_fi::DEFAULT_DENY_SET_CAPACITY`] when absent.
    #[serde(default)]
    pub deny_set_capacity: Option<usize>,
}

/// Token-class discriminant in the issuer config JSON.
#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum TokenClassEnvConfig {
    #[serde(rename = "nip-fi+jwt")]
    DedicatedNipFi,
    #[serde(rename = "at+jwt")]
    AccessTokenAtJwt,
}

// ── NipFiRelayConfig ──────────────────────────────────────────────────────────

/// The relay-level NIP-FI configuration produced by [`NipFiRelayConfig::from_env`].
///
/// Carries the validated `NipFiMode`, the full `IssuerRegistry`, the parallel
/// `IssuerJwksConfig` slice, and the per-issuer S4 command entries.
#[derive(Debug, Clone)]
pub struct NipFiRelayConfig {
    /// The enforcement mode.
    pub mode: NipFiMode,
    /// Validated per-issuer assertion-policy registry.
    pub registry: IssuerRegistry,
    /// Parallel JWKS configs for `ProductionJwksSource`.
    pub jwks_configs: Vec<IssuerJwksConfig>,
    /// Hard upper bound on a single connection lease, in seconds.
    /// `0` when mode is Off/DenyProtected (sentinel: use [`Self::max_connection_lifetime`]).
    pub max_connection_lifetime_secs: u64,
    /// Per-issuer S4 command entries: `(issuer_uri, CommandIssuerEnvConfig)`.
    /// Empty when mode is Off/DenyProtected or no issuer has command fields.
    pub command_configs: Vec<(String, crate::api::nip_fi::CommandIssuerEnvConfig)>,
}

impl NipFiRelayConfig {
    /// Parse NIP-FI relay configuration from the process environment.
    ///
    /// Returns `Err` when `BUZZ_NIP_FI_MODE=enforce` but required config is
    /// missing or invalid (fail-closed).
    pub fn from_env() -> Result<Self, ConfigError> {
        let mode = parse_mode()?;

        if let NipFiMode::Off | NipFiMode::DenyProtected = mode {
            return Ok(Self {
                mode,
                registry: IssuerRegistry::new(),
                jwks_configs: Vec::new(),
                max_connection_lifetime_secs: 0,
                command_configs: Vec::new(),
            });
        }

        // Enforce mode: BUZZ_NIP_FI_ISSUERS and lifetime are required.
        let issuers_json = std::env::var("BUZZ_NIP_FI_ISSUERS").map_err(|_| {
            ConfigError::InvalidValue(
                "BUZZ_NIP_FI_MODE=enforce but BUZZ_NIP_FI_ISSUERS is not set; \
                 set it to a JSON array of issuer configs"
                    .to_string(),
            )
        })?;
        if issuers_json.trim().is_empty() {
            return Err(ConfigError::InvalidValue(
                "BUZZ_NIP_FI_ISSUERS must not be empty in enforce mode".to_string(),
            ));
        }

        let issuer_entries: Vec<IssuerEnvConfig> =
            serde_json::from_str(&issuers_json).map_err(|e| {
                ConfigError::InvalidValue(format!(
                    "BUZZ_NIP_FI_ISSUERS could not be parsed (line {}, column {})",
                    e.line(),
                    e.column()
                ))
            })?;

        if issuer_entries.is_empty() {
            return Err(ConfigError::InvalidValue(
                "BUZZ_NIP_FI_ISSUERS must contain at least one issuer in enforce mode".to_string(),
            ));
        }

        let max_connection_lifetime_secs = parse_u64_bounded(
            "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS",
            1,
            MAX_CONNECTION_LIFETIME_SECS,
        )?
        .ok_or_else(|| {
            ConfigError::InvalidValue(
                "BUZZ_NIP_FI_MODE=enforce but \
                 BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS is not set; \
                 every enforce deployment must configure a positive finite value"
                    .to_string(),
            )
        })?;

        let mut registry = IssuerRegistry::new();
        let mut jwks_configs = Vec::with_capacity(issuer_entries.len());
        let mut command_configs = Vec::new();

        for entry in &issuer_entries {
            let idx = jwks_configs.len(); // 0-based issuer index for error messages
            let (policy, jwks_config) = build_issuer(entry).map_err(|e| {
                ConfigError::InvalidValue(format!("BUZZ_NIP_FI_ISSUERS: issuer [index {idx}]: {e}"))
            })?;
            registry.insert(policy);
            jwks_configs.push(jwks_config);

            // Extract S4 command fields if present.
            if let Some(cmd_age) = entry.maximum_command_age_seconds {
                let principals = entry.authorized_principals.clone().unwrap_or_default();
                // Malformed S4 fields in enforce mode must reject startup.
                if principals.is_empty() {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {idx}]: \
                         maximum_command_age_seconds is set but authorized_principals is \
                         absent or empty — command API requires at least one authorized principal"
                    )));
                }
                if cmd_age == 0 || cmd_age > 60 {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {idx}]: \
                         maximum_command_age_seconds must be in [1, 60]; got {cmd_age}"
                    )));
                }
                let capacity = entry
                    .deny_set_capacity
                    .unwrap_or(crate::api::nip_fi::DEFAULT_DENY_SET_CAPACITY);
                if capacity == 0 {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {idx}]: \
                         deny_set_capacity must be positive (non-zero)"
                    )));
                }
                // Validate that CommandIssuerPolicy can be constructed — this is the
                // same gate the builder uses, so a startup rejection here is tight.
                crate::api::nip_fi::validate_command_issuer_config(
                    idx,
                    cmd_age,
                    &principals,
                    capacity,
                )
                .map_err(ConfigError::InvalidValue)?;
                command_configs.push((
                    entry.issuer.clone(),
                    crate::api::nip_fi::CommandIssuerEnvConfig {
                        maximum_command_age_seconds: Some(cmd_age),
                        authorized_principals: Some(principals),
                        deny_set_capacity: entry.deny_set_capacity,
                    },
                ));
            } else {
                // No maximum_command_age_seconds: in enforce mode every issuer MUST be
                // command-capable (NIP-FI.md:405-409 requires maximum_command_age per
                // authorized issuer).  An enforce issuer without command fields would
                // silently produce an empty command_configs and a permanently-503
                // endpoint — reject it at startup.
                //
                // Orphan S4 fields are detected first to give the operator precise
                // error feedback before the all-or-nothing rejection fires.
                if entry.authorized_principals.is_some() {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {idx}]: \
                         authorized_principals is set but maximum_command_age_seconds is absent — \
                         S4 command API requires maximum_command_age_seconds"
                    )));
                }
                if entry.deny_set_capacity.is_some() {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {idx}]: \
                         deny_set_capacity is set but maximum_command_age_seconds is absent — \
                         S4 command API requires maximum_command_age_seconds"
                    )));
                }
                // No orphan fields: reject because enforce mode requires every issuer
                // to be command-capable (NIP-FI.md:405-409).
                return Err(ConfigError::InvalidValue(format!(
                    "BUZZ_NIP_FI_ISSUERS: issuer [index {idx}]: \
                     maximum_command_age_seconds is required in enforce mode — \
                     every configured issuer must be command-capable. \
                     Add maximum_command_age_seconds and authorized_principals, \
                     or remove this issuer from BUZZ_NIP_FI_ISSUERS"
                )));
            }
        }

        // Delegate final validation to buzz-auth startup gate.
        validate_nip_fi_config(NipFiMode::Enforce, &registry, &jwks_configs).map_err(
            |e: NipFiStartupError| ConfigError::InvalidValue(format!("NIP-FI config invalid: {e}")),
        )?;

        Ok(Self {
            mode,
            registry,
            jwks_configs,
            max_connection_lifetime_secs,
            command_configs,
        })
    }

    /// Returns the configured `max_connection_lifetime` as a `Duration`.
    /// Returns `None` in `Off`/`DenyProtected` mode.
    pub fn max_connection_lifetime(&self) -> Option<std::time::Duration> {
        if self.max_connection_lifetime_secs == 0 {
            None
        } else {
            Some(std::time::Duration::from_secs(
                self.max_connection_lifetime_secs,
            ))
        }
    }

    /// Returns `true` when the relay is in `Enforce` mode.
    pub fn is_enforce(&self) -> bool {
        matches!(self.mode, NipFiMode::Enforce)
    }

    /// Construct an Off-mode config with no environment reads.
    ///
    /// Used by `Config::hermetic_for_test()` to avoid racing against NIP-FI
    /// env-mutating tests in the same process.  Only available in test builds.
    #[cfg(test)]
    pub(crate) fn off_for_test() -> Self {
        Self {
            mode: NipFiMode::Off,
            registry: IssuerRegistry::new(),
            jwks_configs: Vec::new(),
            max_connection_lifetime_secs: 0,
            command_configs: Vec::new(),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_mode() -> Result<NipFiMode, ConfigError> {
    match std::env::var("BUZZ_NIP_FI_MODE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "off" => Ok(NipFiMode::Off),
        "enforce" => Ok(NipFiMode::Enforce),
        "deny_protected" => Ok(NipFiMode::DenyProtected),
        other => Err(ConfigError::InvalidValue(format!(
            "BUZZ_NIP_FI_MODE must be 'off', 'enforce', or 'deny_protected'; got {other:?}"
        ))),
    }
}

fn parse_algorithm(s: &str) -> Result<JwtAlgorithm, String> {
    match s {
        "RS256" => Ok(JwtAlgorithm::RS256),
        "RS384" => Ok(JwtAlgorithm::RS384),
        "RS512" => Ok(JwtAlgorithm::RS512),
        "ES256" => Ok(JwtAlgorithm::ES256),
        "ES384" => Ok(JwtAlgorithm::ES384),
        "PS256" => Ok(JwtAlgorithm::PS256),
        "PS384" => Ok(JwtAlgorithm::PS384),
        "PS512" => Ok(JwtAlgorithm::PS512),
        "EdDSA" => Ok(JwtAlgorithm::EdDSA),
        other => Err(format!("unknown or non-asymmetric algorithm {other:?}")),
    }
}

fn build_issuer(entry: &IssuerEnvConfig) -> Result<(IssuerPolicy, IssuerJwksConfig), String> {
    let algorithms: Vec<JwtAlgorithm> = entry
        .algorithms
        .iter()
        .map(|s| parse_algorithm(s))
        .collect::<Result<_, _>>()?;

    let token_class = match entry.token_class {
        TokenClassEnvConfig::DedicatedNipFi => TokenClass::DedicatedNipFi,
        TokenClassEnvConfig::AccessTokenAtJwt => {
            return Err("\"at+jwt\" token class requires a subject-class contract; \
                 use \"nip-fi+jwt\" for initial deployments"
                .to_string());
        }
    };

    let jwks_contract = JwksSourceContract::new(
        entry.jwks_uri.clone(),
        entry.jwks_refresh_interval_seconds,
        entry.jwks_hard_deadline_seconds,
    )
    .ok_or_else(|| {
        "invalid JWKS source contract (check jwks_uri is HTTPS, \
             refresh_interval < hard_deadline, and both are positive)"
            .to_string()
    })?;

    let policy = IssuerPolicy::new(
        entry.issuer.clone(),
        entry.audiences.clone(),
        token_class,
        FreshnessClass::OfflineJwt,
        algorithms,
        entry.skew_seconds,
        entry.maximum_assertion_age_seconds,
        None, // offline-jwt: no status age
        jwks_contract.clone(),
    )
    .map_err(|e: IssuerPolicyError| e.to_string())?;

    let jwks_config = IssuerJwksConfig {
        issuer: entry.issuer.clone(),
        contract: jwks_contract,
    };

    Ok((policy, jwks_config))
}

fn parse_u64_bounded(var: &str, min: u64, max: u64) -> Result<Option<u64>, ConfigError> {
    match std::env::var(var) {
        Ok(val) => {
            let n: u64 = val.parse().map_err(|_| {
                ConfigError::InvalidValue(format!("{var} must be a positive integer; got {val:?}"))
            })?;
            if n < min || n > max {
                return Err(ConfigError::InvalidValue(format!(
                    "{var} must be between {min} and {max}; got {n}"
                )));
            }
            Ok(Some(n))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidValue(format!(
            "{var} contains invalid UTF-8"
        ))),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: removes env vars on drop to keep tests isolated.
    struct EnvGuard(Vec<&'static str>);
    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            Self(keys.to_vec())
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for key in &self.0 {
                std::env::remove_var(key);
            }
        }
    }

    const NIP_FI_VARS: &[&str] = &[
        "BUZZ_NIP_FI_MODE",
        "BUZZ_NIP_FI_ISSUERS",
        "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS",
    ];

    // ── Mode parsing ──────────────────────────────────────────────────────────

    #[test]
    fn off_mode_requires_no_other_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::remove_var("BUZZ_NIP_FI_MODE");
        let cfg = NipFiRelayConfig::from_env().expect("Off mode must not fail");
        assert!(matches!(cfg.mode, NipFiMode::Off));
        assert!(cfg.registry.is_empty());
    }

    #[test]
    fn deny_protected_requires_no_other_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "deny_protected");
        let cfg = NipFiRelayConfig::from_env().expect("DenyProtected must not fail");
        assert!(matches!(cfg.mode, NipFiMode::DenyProtected));
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "permissive");
        let err = NipFiRelayConfig::from_env().expect_err("unknown mode must error");
        assert!(err.to_string().contains("BUZZ_NIP_FI_MODE"));
    }

    #[test]
    fn enforce_without_issuers_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::remove_var("BUZZ_NIP_FI_ISSUERS");
        let err = NipFiRelayConfig::from_env()
            .expect_err("enforce without issuers must be a config error");
        assert!(err.to_string().contains("BUZZ_NIP_FI_ISSUERS"));
    }

    #[test]
    fn enforce_command_age_without_principals_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // issuer has command age but no principals
        std::env::set_var(
            "BUZZ_NIP_FI_ISSUERS",
            r#"[{
                "issuer": "https://idp.example.com",
                "audiences": ["https://relay.example.com"],
                "token_class": "nip-fi+jwt",
                "algorithms": ["ES256"],
                "maximum_assertion_age_seconds": 3600,
                "jwks_uri": "https://idp.example.com/.well-known/jwks.json",
                "jwks_refresh_interval_seconds": 300,
                "jwks_hard_deadline_seconds": 86400,
                "maximum_command_age_seconds": 30
            }]"#,
        );
        let err = NipFiRelayConfig::from_env()
            .expect_err("command age without principals must fail closed");
        let msg = err.to_string();
        assert!(
            msg.contains("authorized_principals"),
            "error names the missing field: {msg}"
        );
    }

    #[test]
    fn orphan_authorized_principals_without_command_age_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // authorized_principals without maximum_command_age_seconds — orphan field.
        std::env::set_var(
            "BUZZ_NIP_FI_ISSUERS",
            r#"[{
                "issuer": "https://idp.example.com",
                "audiences": ["https://relay.example.com"],
                "token_class": "nip-fi+jwt",
                "algorithms": ["ES256"],
                "maximum_assertion_age_seconds": 3600,
                "jwks_uri": "https://idp.example.com/.well-known/jwks.json",
                "jwks_refresh_interval_seconds": 300,
                "jwks_hard_deadline_seconds": 86400,
                "authorized_principals": ["admin@idp.example.com"]
            }]"#,
        );
        let err = NipFiRelayConfig::from_env()
            .expect_err("orphan authorized_principals must fail closed");
        let msg = err.to_string();
        assert!(
            msg.contains("authorized_principals"),
            "error names the orphan field: {msg}"
        );
        assert!(
            msg.contains("maximum_command_age_seconds"),
            "error names the missing dependency: {msg}"
        );
    }

    #[test]
    fn orphan_deny_set_capacity_without_command_age_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // deny_set_capacity without maximum_command_age_seconds — orphan field.
        std::env::set_var(
            "BUZZ_NIP_FI_ISSUERS",
            r#"[{
                "issuer": "https://idp.example.com",
                "audiences": ["https://relay.example.com"],
                "token_class": "nip-fi+jwt",
                "algorithms": ["ES256"],
                "maximum_assertion_age_seconds": 3600,
                "jwks_uri": "https://idp.example.com/.well-known/jwks.json",
                "jwks_refresh_interval_seconds": 300,
                "jwks_hard_deadline_seconds": 86400,
                "deny_set_capacity": 1000
            }]"#,
        );
        let err =
            NipFiRelayConfig::from_env().expect_err("orphan deny_set_capacity must fail closed");
        let msg = err.to_string();
        assert!(
            msg.contains("deny_set_capacity"),
            "error names the orphan field: {msg}"
        );
        assert!(
            msg.contains("maximum_command_age_seconds"),
            "error names the missing dependency: {msg}"
        );
    }

    #[test]
    fn enforce_issuer_without_command_fields_is_rejected() {
        // An enforce-mode issuer entry with ALL THREE S4 fields absent must
        // fail startup.  This is the blocker-4a case: the issuer is a valid
        // JWKS/assertion issuer but carries no command config.  Without this
        // rejection from_env() would succeed with an empty command_configs,
        // the endpoint would permanently return 503, and startup would log nothing.
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // All three S4 command fields absent — pure assertion/JWKS issuer.
        std::env::set_var(
            "BUZZ_NIP_FI_ISSUERS",
            r#"[{
                "issuer": "https://idp.example.com",
                "audiences": ["https://relay.example.com"],
                "token_class": "nip-fi+jwt",
                "algorithms": ["ES256"],
                "maximum_assertion_age_seconds": 3600,
                "jwks_uri": "https://idp.example.com/.well-known/jwks.json",
                "jwks_refresh_interval_seconds": 300,
                "jwks_hard_deadline_seconds": 86400
            }]"#,
        );
        let err = NipFiRelayConfig::from_env()
            .expect_err("enforce issuer without command fields must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("maximum_command_age_seconds"),
            "error must name the missing field: {msg}"
        );
    }

    // ── Privacy: config error must not expose sensitive values ────────────────
    //
    // Verifies that a malformed BUZZ_NIP_FI_ISSUERS whose authorized_principals
    // contains a sensitive email address does NOT appear in the error message.
    //
    // Mandatory red: restoring the raw serde interpolation (`{e}`) exposes the
    // sentinel value in the Display output and fails this test.

    #[test]
    fn config_error_does_not_expose_sensitive_principal_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        // A syntactically broken JSON object that contains a sensitive sentinel
        // where authorized_principals would be.  The `INVALID_TYPE_HERE` string
        // is not valid JSON for the Vec<String> field — serde will produce a
        // type error that in a naive `{e}` interpolation would include the raw
        // string, potentially exposing the surrounding value.
        const SENTINEL: &str = "admin+private-sentinel@example.invalid";

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // The authorized_principals field is a string instead of an array,
        // which causes serde to emit a type-error that typically includes the
        // supplied value when formatted with `{e}` (the bug we are guarding).
        std::env::set_var(
            "BUZZ_NIP_FI_ISSUERS",
            format!(
                r#"[{{
                    "issuer": "https://idp.example.com",
                    "audiences": ["https://relay.example.com"],
                    "token_class": "nip-fi+jwt",
                    "algorithms": ["ES256"],
                    "maximum_assertion_age_seconds": 3600,
                    "jwks_uri": "https://idp.example.com/.well-known/jwks.json",
                    "jwks_refresh_interval_seconds": 300,
                    "jwks_hard_deadline_seconds": 86400,
                    "maximum_command_age_seconds": 30,
                    "authorized_principals": "{SENTINEL}"
                }}]"#
            ),
        );

        let err = NipFiRelayConfig::from_env().expect_err("malformed issuers must fail");
        let display_msg = err.to_string();
        let debug_msg = format!("{err:?}");

        // Safe category must be present.
        assert!(
            display_msg.contains("BUZZ_NIP_FI_ISSUERS could not be parsed"),
            "Display message must contain the safe category string: {display_msg}"
        );
        // Sentinel must NOT appear in any user-facing output path.
        assert!(
            !display_msg.contains(SENTINEL),
            "Display message must NOT contain the sensitive sentinel: {display_msg}"
        );
        assert!(
            !debug_msg.contains(SENTINEL),
            "Debug output must NOT contain the sensitive sentinel: {debug_msg}"
        );
    }
}
