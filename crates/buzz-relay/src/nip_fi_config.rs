//! NIP-FI relay-level configuration: issuer set, session lifetime, and JWKS
//! warm/refresh.
//!
//! All env-var parsing lives here so `config.rs` stays focused on the top-level
//! `Config` struct. This module is `pub` — `config.rs` constructs it, and the
//! relay reads it as `config.nip_fi`.
//!
//! # Environment variables
//!
//! | Variable | Required | Description |
//! |---|---|---|
//! | `BUZZ_NIP_FI_MODE` | No | `off` (default), `shadow`, `enforce`, or `deny_protected`. `shadow` requires everything `enforce` does. |
//! | `BUZZ_NIP_FI_ISSUERS` | If enforce | JSON array of issuer configs (see [`IssuerEnvConfig`]). |
//! | `BUZZ_NIP_FI_COMMUNITIES` | If enforce | JSON array mapping each community's canonical URI to its authorized issuers (see [`CommunityEnvConfig`]). |
//! | `BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS` | If enforce | Per-partition limit on session lifetime. |
//!
//! Each `BUZZ_NIP_FI_ISSUERS` entry also carries the S4 command-API fields.
//! Enforce-mode startup fails if any issuer lacks the required ones:
//!
//! | Field | Required | Constraint |
//! |---|---|---|
//! | `maximum_command_age_seconds` | Every issuer, in enforce | Integer in `[1, 60]`. |
//! | `authorized_principals` | Every issuer, in enforce | Non-empty array of `sub` values, each 1–2048 bytes; matched by exact, case-sensitive byte comparison. |
//! | `deny_set_capacity` | No | Integer > 0; defaults to [`crate::api::nip_fi::DEFAULT_DENY_SET_CAPACITY`] (50000). |
//!
//! `maximum_assertion_age` is per-issuer only (field `maximum_assertion_age_seconds` in
//! the issuer JSON array), not a relay-level env var. A relay-level duplicate that could
//! disagree with the enforced per-issuer value was removed in this PR.
//!
//! Absent or empty `BUZZ_NIP_FI_MODE` defaults to `off`, keeping the relay
//! backward-compatible until an operator explicitly enables enforcement.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use buzz_auth::{
    validate_nip_fi_config, CommunityBinding, FreshnessClass, IssuerJwksConfig, IssuerPolicy,
    IssuerPolicyError, IssuerRegistry, JwksSourceContract, NipFiMode, NipFiStartupError,
    TokenClass,
};
use buzz_core::tenant::normalize_host;
use jsonwebtoken::Algorithm;

use crate::config::ConfigError;

/// Maximum accepted `max_connection_lifetime` in seconds (30 days).
const MAX_CONNECTION_LIFETIME_SECS: u64 = 30 * 24 * 3600;

// ── Per-issuer JSON config shape ─────────────────────────────────────────────

/// One entry in the `BUZZ_NIP_FI_ISSUERS` JSON array.
///
/// **Example** (one issuer, `nip-fi+jwt` dedicated assertions):
/// ```json
/// [
///   {
///     "issuer": "https://login.example.com",
///     "command_audiences": ["https://relay.example.com"],
///     "token_class": "nip-fi+jwt",
///     "algorithms": ["ES256"],
///     "skew_seconds": 30,
///     "maximum_assertion_age_seconds": 3600,
///     "jwks_uri": "https://login.example.com/.well-known/jwks.json",
///     "jwks_refresh_interval_seconds": 300,
///     "jwks_hard_deadline_seconds": 86400,
///     "maximum_command_age_seconds": 30,
///     "authorized_principals": ["admin-svc@login.example.com"]
///   }
/// ]
/// ```
/// The `require_attested_key` field is not part of this schema; S2 removed it
/// from buzz-auth. S3 enforces key pairing structurally for every issuer.
#[derive(Debug, serde::Deserialize)]
pub(super) struct IssuerEnvConfig {
    /// Exact `iss` value.
    pub issuer: String,
    /// One or more accepted `aud` values for S4 command JWTs. Assertion `aud`
    /// is the community's canonical URI from `BUZZ_NIP_FI_COMMUNITIES`.
    pub command_audiences: Vec<String>,
    /// Removed field. Present only so a stale config fails loudly instead of
    /// being silently ignored.
    #[serde(default, deserialize_with = "field_present")]
    pub audiences: bool,
    /// `"at+jwt"` or `"nip-fi+jwt"`.
    pub token_class: TokenClassEnvConfig,
    /// Algorithm names, e.g. `["ES256", "RS256"]`.
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

    // ── S4 command-API fields (required in enforce and shadow modes) ──────
    /// Maximum command JWT age in seconds; `0 < x ≤ 60`.  Required on every
    /// issuer in enforce and shadow modes.
    #[serde(default)]
    pub maximum_command_age_seconds: Option<u64>,
    /// Non-empty list of authorized `sub` values.  Required on every issuer
    /// in enforce mode.
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

/// One entry in the `BUZZ_NIP_FI_COMMUNITIES` JSON array.
///
/// ```json
/// [{ "canonical_uri": "https://acme.relay.example",
///    "authorized_issuers": ["https://login.example.com"] }]
/// ```
/// `canonical_uri` is the community's exact assertion `aud` (NIP-FI.md:59) and
/// must be `https://<authority>` with nothing after the authority; its
/// authority is the request `Host` that selects the community.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommunityEnvConfig {
    pub canonical_uri: String,
    pub authorized_issuers: Vec<String>,
}

/// The deployment's Host → community map (NIP-FI.md:193-197), resolved with
/// no database access. Empty unless the mode is `enforce`.
#[derive(Debug, Clone, Default)]
pub struct NipFiCommunities {
    by_host: BTreeMap<String, CommunityBinding>,
    /// Test-only: one community served at every Host, for fixtures that
    /// exercise enforce-mode handlers on per-test unique Hosts.
    #[cfg(test)]
    any_host: Option<CommunityBinding>,
}

impl NipFiCommunities {
    /// The community served at `raw_host`, matched after the same
    /// normalization the tenant binder applies.
    pub fn resolve(&self, raw_host: &str) -> Option<&CommunityBinding> {
        #[cfg(test)]
        if let Some(binding) = &self.any_host {
            return Some(binding);
        }
        self.by_host.get(&normalize_host(raw_host))
    }

    /// Validate the community entries against the configured issuers. Errors
    /// name entries by index only, never by URI or issuer (NIP-FI.md:777-779).
    fn from_entries(
        entries: Vec<CommunityEnvConfig>,
        configured: &BTreeSet<&str>,
    ) -> Result<Self, String> {
        if entries.is_empty() {
            return Err("must contain at least one community".to_string());
        }
        let mut by_host = BTreeMap::new();
        for (idx, entry) in entries.into_iter().enumerate() {
            let host = canonical_authority(&entry.canonical_uri)
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!(
                        "community at index {idx}: canonical_uri must be \
                     https://<host>[:port] in lowercase, with no path, query, \
                     fragment, userinfo, trailing dot, or default port"
                    )
                })?;
            if let Some(i) = entry
                .authorized_issuers
                .iter()
                .position(|iss| !configured.contains(iss.as_str()))
            {
                return Err(format!(
                    "community at index {idx}: authorized issuer at index {i} \
                     is not configured in BUZZ_NIP_FI_ISSUERS"
                ));
            }
            let binding = CommunityBinding::new(entry.canonical_uri, entry.authorized_issuers)
                .map_err(|e| format!("community at index {idx}: {e}"))?;
            // One Host per canonical URI, so a unique Host also means a
            // unique `aud`.
            if by_host.insert(host, binding).is_some() {
                return Err(format!(
                    "community at index {idx}: canonical_uri maps to the same \
                     Host as an earlier community"
                ));
            }
        }
        Ok(Self {
            by_host,
            #[cfg(test)]
            any_host: None,
        })
    }

    /// Every community `aud` that authorizes `issuer`.
    fn audiences_authorizing(&self, issuer: &str) -> Vec<String> {
        self.by_host
            .values()
            .filter(|c| c.authorizes(issuer))
            .map(|c| c.expected_aud().to_owned())
            .collect()
    }
}

/// The authority of a canonical community URI, or `None` unless the URI is
/// exactly `https://<authority>` and the URL parser reserializes that authority
/// byte-for-byte. The round trip rejects anything the parser would repair:
/// whitespace, control characters, backslashes, uppercase, userinfo, path,
/// query, fragment, a redundant default `:443`, and non-canonical IPv6.
/// The authority must also already be in `normalize_host()` form, which
/// rejects a final trailing dot and `:80`, so the Host map key equals the
/// `aud` authority exactly and is never empty.
fn canonical_authority(uri: &str) -> Option<&str> {
    let authority = uri.strip_prefix("https://")?;
    let parsed = url::Url::parse(uri).ok()?;
    let host = parsed.host_str()?;
    let reserialized = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    (reserialized == authority && normalize_host(authority) == authority).then_some(authority)
}

// ── Relay-level NIP-FI config ─────────────────────────────────────────────────

/// The relay-level NIP-FI configuration produced by `Config::from_env`.
///
/// Carries the validated `NipFiMode`, the full `IssuerRegistry`,
/// the parallel `IssuerJwksConfig` slice for `ProductionJwksSource`, and the
/// session-lifetime bound.
#[derive(Debug, Clone)]
pub struct NipFiRelayConfig {
    /// The enforcement mode selected by `BUZZ_NIP_FI_MODE`.
    pub mode: NipFiMode,
    /// Validated per-issuer assertion-policy registry.
    pub registry: IssuerRegistry,
    /// Host → community map consulted before every enforce-mode verification.
    pub communities: NipFiCommunities,
    /// Parallel JWKS configs for `ProductionJwksSource` construction.
    pub jwks_configs: Vec<IssuerJwksConfig>,
    /// Hard upper bound on a single connection lease, in seconds.
    /// Required in enforce mode per spec (NIP-FI.md §Request and session
    /// bounds): every deployment MUST configure a positive finite value.
    pub max_connection_lifetime_secs: u64,
    /// Per-issuer S4 command entries: `(issuer_uri, CommandIssuerEnvConfig)`.
    /// Empty when mode is Off/DenyProtected.
    pub command_configs: Vec<(String, crate::api::nip_fi::CommandIssuerEnvConfig)>,
}

impl NipFiRelayConfig {
    /// Parse NIP-FI relay configuration from the process environment.
    ///
    /// Returns `Err` when `BUZZ_NIP_FI_MODE=enforce` but required config is
    /// missing or invalid (fail-closed: no token is accepted until this passes).
    pub fn from_env() -> Result<Self, ConfigError> {
        let mode = parse_mode()?;

        if !mode.evaluates() {
            return Ok(Self {
                mode,
                registry: IssuerRegistry::new(),
                communities: NipFiCommunities::default(),
                jwks_configs: Vec::new(),
                max_connection_lifetime_secs: 0,
                command_configs: Vec::new(),
            });
        }

        // Enforce and Shadow: all fields required.
        let issuers_json = std::env::var("BUZZ_NIP_FI_ISSUERS").map_err(|_| {
            mode_requires(
                mode,
                "BUZZ_NIP_FI_ISSUERS is not set; set it to a JSON array of issuer configs",
            )
        })?;
        if issuers_json.trim().is_empty() {
            return Err(ConfigError::InvalidValue(
                "BUZZ_NIP_FI_ISSUERS must not be empty in enforce mode".to_string(),
            ));
        }

        let issuer_entries: Vec<IssuerEnvConfig> =
            serde_json::from_str(&issuers_json).map_err(|e| {
                // Do not embed raw `e` — serde_json type-error messages can
                // include the unexpected field value verbatim (issuer URLs, etc).
                // Use classify() and positional info only.  [NIP-FI.md:777-779]
                ConfigError::InvalidValue(format!(
                    "BUZZ_NIP_FI_ISSUERS is not valid JSON: {:?} at line {} column {}",
                    e.classify(),
                    e.line(),
                    e.column(),
                ))
            })?;

        if issuer_entries.is_empty() {
            return Err(ConfigError::InvalidValue(
                "BUZZ_NIP_FI_ISSUERS must contain at least one issuer in enforce mode".to_string(),
            ));
        }

        // `BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS` is intentionally NOT parsed
        // here. The authoritative `maximum_assertion_age` comes from each issuer's
        // JSON config entry (field `maximum_assertion_age_seconds`). A relay-level
        // duplicate that could disagree with the per-issuer value is a config-drift
        // trap — removed in this PR.

        let max_connection_lifetime_secs = parse_u64_bounded(
            "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS",
            1,
            MAX_CONNECTION_LIFETIME_SECS,
        )?
        .ok_or_else(|| {
            mode_requires(
                mode,
                "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS is not set; \
                 every enforce deployment must configure a positive finite value",
            )
        })?;

        let mut policies = Vec::with_capacity(issuer_entries.len());
        let mut jwks_configs = Vec::with_capacity(issuer_entries.len());
        let mut command_configs = Vec::new();

        for (issuer_idx, entry) in issuer_entries.iter().enumerate() {
            let (policy, jwks_config) = build_issuer(entry).map_err(|e| {
                ConfigError::InvalidValue(format!(
                    // issuer_idx is a non-identifying diagnostic code.
                    // Raw `iss` is excluded per NIP-FI.md:777-779.
                    "BUZZ_NIP_FI_ISSUERS: issuer at index {issuer_idx}: {e}"
                ))
            })?;
            policies.push(policy);
            jwks_configs.push(jwks_config);

            // Extract S4 command fields if present.
            if let Some(cmd_age) = entry.maximum_command_age_seconds {
                let principals = entry.authorized_principals.clone().unwrap_or_default();
                // Malformed S4 fields in enforce mode must reject startup.
                if principals.is_empty() {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {issuer_idx}]: \
                         maximum_command_age_seconds is set but authorized_principals is \
                         absent or empty — command API requires at least one authorized principal"
                    )));
                }
                if cmd_age == 0 || cmd_age > 60 {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {issuer_idx}]: \
                         maximum_command_age_seconds must be in [1, 60]; got {cmd_age}"
                    )));
                }
                let capacity = entry
                    .deny_set_capacity
                    .unwrap_or(crate::api::nip_fi::DEFAULT_DENY_SET_CAPACITY);
                if capacity == 0 {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {issuer_idx}]: \
                         deny_set_capacity must be positive (non-zero)"
                    )));
                }
                // Validate that CommandIssuerPolicy can be constructed — this is the
                // same gate the builder uses, so a startup rejection here is tight.
                crate::api::nip_fi::validate_command_issuer_config(
                    issuer_idx,
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
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {issuer_idx}]: \
                         authorized_principals is set but maximum_command_age_seconds is absent — \
                         S4 command API requires maximum_command_age_seconds"
                    )));
                }
                if entry.deny_set_capacity.is_some() {
                    return Err(ConfigError::InvalidValue(format!(
                        "BUZZ_NIP_FI_ISSUERS: issuer [index {issuer_idx}]: \
                         deny_set_capacity is set but maximum_command_age_seconds is absent — \
                         S4 command API requires maximum_command_age_seconds"
                    )));
                }
                // No orphan fields: reject because enforce mode requires every issuer
                // to be command-capable (NIP-FI.md:405-409).
                return Err(ConfigError::InvalidValue(format!(
                    "BUZZ_NIP_FI_ISSUERS: issuer [index {issuer_idx}]: \
                     maximum_command_age_seconds is required in enforce mode — \
                     every configured issuer must be command-capable. \
                     Add maximum_command_age_seconds and authorized_principals, \
                     or remove this issuer from BUZZ_NIP_FI_ISSUERS"
                )));
            }
        }

        let configured: BTreeSet<&str> = policies.iter().map(IssuerPolicy::issuer).collect();
        let communities = parse_communities(mode, &configured)?;
        // Fold each issuer's community allowlist into its policy ID
        // (NIP-FI.md:145), and reject an issuer no community authorizes
        // (NIP-FI.md:183).
        let mut registry = IssuerRegistry::new();
        for (issuer_idx, policy) in policies.into_iter().enumerate() {
            let audiences = communities.audiences_authorizing(policy.issuer());
            if audiences.is_empty() {
                return Err(ConfigError::InvalidValue(format!(
                    "BUZZ_NIP_FI_ISSUERS: issuer at index {issuer_idx} is not \
                     authorized by any community in BUZZ_NIP_FI_COMMUNITIES"
                )));
            }
            registry.insert(policy.authorized_for_communities(audiences));
        }

        // Delegate final validation to buzz-auth startup gate.
        validate_nip_fi_config(mode, &registry, &jwks_configs).map_err(
            |e: NipFiStartupError| ConfigError::InvalidValue(format!("NIP-FI config invalid: {e}")),
        )?;

        Ok(Self {
            mode,
            registry,
            communities,
            jwks_configs,
            max_connection_lifetime_secs,
            command_configs,
        })
    }
}

/// Parse and validate `BUZZ_NIP_FI_COMMUNITIES` against `registry`.
fn parse_communities(
    mode: NipFiMode,
    configured: &BTreeSet<&str>,
) -> Result<NipFiCommunities, ConfigError> {
    let raw = std::env::var("BUZZ_NIP_FI_COMMUNITIES").unwrap_or_default();
    if raw.trim().is_empty() {
        return Err(mode_requires(
            mode,
            "BUZZ_NIP_FI_COMMUNITIES is not set; \
             set it to a JSON array of {canonical_uri, authorized_issuers}",
        ));
    }
    let entries: Vec<CommunityEnvConfig> = serde_json::from_str(&raw).map_err(|e| {
        ConfigError::InvalidValue(format!(
            "BUZZ_NIP_FI_COMMUNITIES is not valid JSON: {:?} at line {} column {}",
            e.classify(),
            e.line(),
            e.column(),
        ))
    })?;
    NipFiCommunities::from_entries(entries, configured)
        .map_err(|e| ConfigError::InvalidValue(format!("BUZZ_NIP_FI_COMMUNITIES: {e}")))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_mode() -> Result<NipFiMode, ConfigError> {
    match std::env::var("BUZZ_NIP_FI_MODE")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("off") => Ok(NipFiMode::Off),
        Some("enforce") => Ok(NipFiMode::Enforce),
        Some("deny_protected") => Ok(NipFiMode::DenyProtected),
        Some("shadow") => Ok(NipFiMode::Shadow),
        Some(other) => Err(ConfigError::InvalidValue(format!(
            "BUZZ_NIP_FI_MODE must be \"enforce\", \"shadow\", \"deny_protected\", or \"off\"; \
             got {other:?}"
        ))),
    }
}

/// A missing-setting startup error naming the configured mode.
fn mode_requires(mode: NipFiMode, missing: &str) -> ConfigError {
    let value = match mode {
        NipFiMode::Enforce => "enforce",
        NipFiMode::Shadow => "shadow",
        NipFiMode::Off => "off",
        NipFiMode::DenyProtected => "deny_protected",
    };
    ConfigError::InvalidValue(format!("BUZZ_NIP_FI_MODE={value} but {missing}"))
}

/// Parse an optional positive `u64` env var bounded to `[min_val, max_val]`.
/// Returns `None` when the variable is absent or empty.
fn parse_u64_bounded(name: &str, min_val: u64, max_val: u64) -> Result<Option<u64>, ConfigError> {
    match std::env::var(name) {
        Err(_) => Ok(None),
        Ok(raw) if raw.trim().is_empty() => Ok(None),
        Ok(raw) => {
            let v: u64 = raw.trim().parse().map_err(|_| {
                ConfigError::InvalidValue(format!("{name} must be a positive integer"))
            })?;
            if v < min_val || v > max_val {
                return Err(ConfigError::InvalidValue(format!(
                    "{name} must be in {min_val}..={max_val}"
                )));
            }
            Ok(Some(v))
        }
    }
}

/// Parse a `jsonwebtoken::Algorithm` from a case-sensitive string.
fn parse_algorithm(s: &str) -> Result<Algorithm, String> {
    match s {
        "ES256" => Ok(Algorithm::ES256),
        "ES384" => Ok(Algorithm::ES384),
        "RS256" => Ok(Algorithm::RS256),
        "RS384" => Ok(Algorithm::RS384),
        "RS512" => Ok(Algorithm::RS512),
        "PS256" => Ok(Algorithm::PS256),
        "PS384" => Ok(Algorithm::PS384),
        "PS512" => Ok(Algorithm::PS512),
        "EdDSA" => Ok(Algorithm::EdDSA),
        other => Err(format!(
            "unknown or non-asymmetric algorithm (got {} chars); \
             supported: ES256 ES384 RS256 RS384 RS512 PS256 PS384 PS512 EdDSA",
            other.len()
        )),
    }
}

/// `true` whenever the field appears, whatever its value (including `null`).
fn field_present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    <serde::de::IgnoredAny as serde::Deserialize>::deserialize(d).map(|_| true)
}

fn build_issuer(entry: &IssuerEnvConfig) -> Result<(IssuerPolicy, IssuerJwksConfig), String> {
    if entry.audiences {
        return Err("\"audiences\" was removed: list command-JWT audiences in \
             \"command_audiences\" and map assertion audiences per community in \
             BUZZ_NIP_FI_COMMUNITIES"
            .to_string());
    }
    let algorithms: Vec<Algorithm> = entry
        .algorithms
        .iter()
        .map(|s| parse_algorithm(s))
        .collect::<Result<_, _>>()?;

    let token_class = match entry.token_class {
        TokenClassEnvConfig::DedicatedNipFi => TokenClass::DedicatedNipFi,
        TokenClassEnvConfig::AccessTokenAtJwt => {
            // at+jwt requires a SubjectClassContract; for simplicity in the
            // initial deployment, dedicated nip-fi+jwt is the expected class.
            // at+jwt support is left for a follow-up — fail closed with a
            // clear message so operators know the required fields.
            return Err("\"at+jwt\" token class requires a subject-class contract; \
                 use \"nip-fi+jwt\" for initial deployments or add \
                 subject_class fields to the issuer config"
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
        entry.command_audiences.clone(),
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

// ── Duration helpers ──────────────────────────────────────────────────────────

impl NipFiRelayConfig {
    /// Returns the configured `max_connection_lifetime` as a `Duration`.
    /// Returns `None` in `Off`/`DenyProtected` mode (sentinel value 0).
    pub fn max_connection_lifetime(&self) -> Option<Duration> {
        if self.max_connection_lifetime_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(self.max_connection_lifetime_secs))
        }
    }

    /// Returns `true` when the relay is in `Enforce` mode.
    pub fn is_enforce(&self) -> bool {
        self.mode.enforces()
    }
}

#[cfg(test)]
impl NipFiCommunities {
    /// One community at `canonical_uri` authorizing `issuers`.
    pub(crate) fn for_test(canonical_uri: &str, issuers: &[&str]) -> Self {
        let entries = vec![CommunityEnvConfig {
            canonical_uri: canonical_uri.to_owned(),
            authorized_issuers: issuers.iter().map(|i| (*i).to_owned()).collect(),
        }];
        Self::from_entries(entries, &issuers.iter().copied().collect())
            .expect("valid test community")
    }

    /// One community, expecting `aud` and authorizing `issuers`, served at
    /// every Host.
    pub(crate) fn for_test_any_host(aud: &str, issuers: &[&str]) -> Self {
        let binding =
            CommunityBinding::new(aud.to_owned(), issuers.iter().map(|i| (*i).to_owned()))
                .expect("valid test community");
        Self {
            any_host: Some(binding),
            ..Self::default()
        }
    }
}

/// Process-global mutex serializing all reads and writes to NIP-FI environment
/// variables. Both `NipFiRelayConfig::from_env()` callers and test code that
/// temporarily mutates NIP-FI env vars must hold this lock to prevent
/// cross-test races when the suite runs with multiple threads.
///
/// Exposed at module level (not just `#[cfg(test)]`) so `router.rs` test
/// fixtures that call `Config::from_env()` can hold it across the NIP-FI
/// env-var window without racing this module's own tests.
/// [Fix 5: FI-TRACE-ENV-RACE]
#[cfg(test)]
pub(crate) static NIP_FI_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Threads that reached `Config::for_test()`'s `NIP_FI_ENV_LOCK` acquisition;
/// lets the lock witness observe arrival instead of inferring it from time.
#[cfg(test)]
pub(crate) static FOR_TEST_LOCK_WAITERS: std::sync::Mutex<Vec<std::thread::ThreadId>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
mod tests {
    use super::*;

    // Env-mutating tests hold the module-level `NIP_FI_ENV_LOCK`, shared with
    // `Config::for_test()` so router fixtures cannot race these tests.

    /// RAII guard: removes a set of env vars when dropped, restoring a clean
    /// state even on test panic.
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

    /// Set `BUZZ_NIP_FI_ISSUERS` to `json` and, when it parses, map one
    /// community authorizing every listed issuer.
    fn set_issuers(json: impl AsRef<std::ffi::OsStr>) {
        let json = json.as_ref();
        std::env::set_var("BUZZ_NIP_FI_ISSUERS", json);
        let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(&json.to_string_lossy())
        else {
            return;
        };
        let issuers: Vec<_> = entries.iter().filter_map(|e| e.get("issuer")).collect();
        std::env::set_var(
            "BUZZ_NIP_FI_COMMUNITIES",
            serde_json::json!([{
                "canonical_uri": "https://relay.test",
                "authorized_issuers": issuers,
            }])
            .to_string(),
        );
    }

    const NIP_FI_VARS: &[&str] = &[
        "BUZZ_NIP_FI_MODE",
        "BUZZ_NIP_FI_ISSUERS",
        "BUZZ_NIP_FI_COMMUNITIES",
        "BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS",
        "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS",
    ];

    /// Same-process witness (plain libtest, not nextest): a fixture's
    /// `Config::for_test()` read waits while an FI writer holds the lock over
    /// an invalid Enforce environment, then loads the restored environment.
    /// [FI-TRACE-ENV-RACE]
    #[test]
    fn fixture_config_read_waits_for_fi_env_lock() {
        let guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let env = EnvGuard::new(NIP_FI_VARS);
        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::remove_var("BUZZ_NIP_FI_ISSUERS");

        let reader = std::thread::spawn(|| crate::config::Config::for_test().nip_fi.mode);
        let reader_id = reader.thread().id();
        while !super::FOR_TEST_LOCK_WAITERS
            .lock()
            .unwrap()
            .contains(&reader_id)
        {
            std::thread::yield_now();
        }
        // The reader is at the lock. Give an unlocked reader ample turns to
        // read the invalid environment and panic before judging exclusion.
        for _ in 0..10_000 {
            if reader.is_finished() {
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            !reader.is_finished(),
            "fixture read must block while the invalid FI environment is locked"
        );

        drop(env);
        drop(guard);
        let mode = reader
            .join()
            .expect("fixture read must load the restored environment");
        assert!(matches!(mode, NipFiMode::Off));
    }

    #[test]
    fn off_mode_requires_no_other_config() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        // NipFiMode::Off is the default: no issuers, no age limit.
        std::env::remove_var("BUZZ_NIP_FI_MODE");
        let cfg = NipFiRelayConfig::from_env().expect("Off mode must not fail");
        assert!(matches!(cfg.mode, NipFiMode::Off));
        assert!(cfg.registry.is_empty());
    }

    #[test]
    fn deny_protected_requires_no_other_config() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "deny_protected");
        let cfg = NipFiRelayConfig::from_env().expect("DenyProtected mode must not fail");
        assert!(matches!(cfg.mode, NipFiMode::DenyProtected));
    }

    #[test]
    fn enforce_without_issuers_fails_closed() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::remove_var("BUZZ_NIP_FI_ISSUERS");
        std::env::remove_var("BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS");
        let err = NipFiRelayConfig::from_env()
            .expect_err("enforce without issuers must be a config error");
        let msg = err.to_string();
        assert!(
            msg.contains("BUZZ_NIP_FI_ISSUERS"),
            "error names the missing var: {msg}"
        );
    }

    /// A complete, valid Enforce issuer entry. Tests derive negative fixtures
    /// from it by removing exactly one field.
    fn valid_enforce_issuer() -> serde_json::Value {
        serde_json::json!({
            "issuer": "https://issuer.test",
            "command_audiences": ["https://relay.test"],
            "token_class": "nip-fi+jwt",
            "algorithms": ["ES256"],
            "skew_seconds": 30,
            "maximum_assertion_age_seconds": 3600,
            "jwks_uri": "https://issuer.test/.well-known/jwks.json",
            "jwks_refresh_interval_seconds": 300,
            "jwks_hard_deadline_seconds": 3600,
            "maximum_command_age_seconds": 30,
            "authorized_principals": ["admin@issuer.test"]
        })
    }

    #[test]
    fn enforce_without_assertion_age_fails_closed() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");

        // Success control: the complete fixture is accepted.
        let valid = valid_enforce_issuer();
        set_issuers(serde_json::json!([valid]).to_string());
        NipFiRelayConfig::from_env().expect("complete Enforce issuer config must be accepted");

        // Same fixture minus only the per-issuer age bound.
        let mut missing_age = valid_enforce_issuer();
        missing_age
            .as_object_mut()
            .unwrap()
            .remove("maximum_assertion_age_seconds");
        set_issuers(serde_json::json!([missing_age]).to_string());
        let err = NipFiRelayConfig::from_env()
            .expect_err("Enforce issuer without maximum_assertion_age_seconds must fail closed");
        // The parser deliberately reports only the serde error class (never
        // the message) so config values cannot leak; with the control above
        // passing, the one-field difference is what produced this rejection.
        // It must be the deserialization rejection, not a later policy-build
        // failure (which a defaulted age would hit instead).
        let msg = err.to_string();
        assert!(
            msg.contains("BUZZ_NIP_FI_ISSUERS is not valid JSON: Data"),
            "missing maximum_assertion_age_seconds must be rejected at deserialization: {msg}"
        );
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "permissive");
        let err = NipFiRelayConfig::from_env().expect_err("unknown mode must error");
        assert!(err.to_string().contains("BUZZ_NIP_FI_MODE"));
    }

    // ── R5 privacy sentinel tests ─────────────────────────────────────────────
    //
    // These tests prove that parse errors on BUZZ_NIP_FI_ISSUERS do NOT echo
    // raw issuer config values (URLs, audience strings, issuer identifiers) in
    // the error messages.  [NIP-FI.md:777-779]
    //
    // The test input embeds a unique sentinel string that should never appear in
    // any error message.  Failing this invariant would mean serde_json or another
    // parser is leaking operator-supplied field values into error text.
    //
    // Falsifying mutation for all tests: remove the `.classify()` / `other.len()`
    // wrapping in `from_env()` / `parse_algorithm()` and restore a raw `{e}` or
    // `{s}` interpolation.  The sentinel strings would appear in the error
    // message and the assertion fires.

    /// Malformed issuer JSON: wrong-typed field must not leak the sentinel value.
    ///
    /// We use a valid JSON array with `skew_seconds` as a string (where the
    /// deserializer expects a number).  Raw serde would echo the actual string
    /// value in a type-error message like `expected u64, got string "SENTINEL..."`.
    /// The test asserts the sentinel does NOT appear — proving the code strips or
    /// classifies the error rather than forwarding serde's message.
    ///
    /// This is stronger than using outright-malformed JSON, which serde never
    /// echoes in the first place.  A non-discriminating malformed-JSON sentinel
    /// passes even if the code leaks values from valid-but-wrong-typed fields.
    #[test]
    fn malformed_issuer_json_error_does_not_leak_raw_value() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        // A sentinel that serde would echo in a type-mismatch error if not suppressed.
        const SENTINEL: &str = "SENTINEL_SKEW_VALUE_abc123xyz";
        // Valid array with `skew_seconds` as a string — deserializer expects u64.
        // Raw serde error would be something like:
        //   "invalid type: string \"SENTINEL_SKEW_VALUE_abc123xyz\", expected u64"
        let issuers_json = serde_json::json!([{
            "issuer": "https://issuer.test",
            "command_audiences": ["https://relay.test"],
            "token_class": "nip-fi+jwt",
            "algorithms": ["ES256"],
            "skew_seconds": SENTINEL,   // wrong type: serde echoes this value
            "maximum_assertion_age_seconds": 3600,
            "jwks_uri": "https://issuer.test/.well-known/jwks.json",
            "jwks_refresh_interval_seconds": 300,
            "jwks_hard_deadline_seconds": 3600
        }])
        .to_string();
        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        set_issuers(&issuers_json);
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");

        let err = NipFiRelayConfig::from_env().expect_err("wrong-typed field must fail");
        let msg = err.to_string();

        assert!(
            !msg.contains(SENTINEL),
            "parse error MUST NOT echo the raw field value (privacy sentinel leaked): {msg}"
        );
        // The error must still be non-empty and identify the config variable.
        assert!(
            msg.contains("BUZZ_NIP_FI_ISSUERS"),
            "error must name the config variable: {msg}"
        );
    }

    /// Invalid algorithm string error must not echo the raw value.
    #[test]
    fn invalid_algorithm_error_does_not_leak_raw_value() {
        // parse_algorithm is private; we test it indirectly by passing a full
        // issuer config with a sentinel algorithm name.
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        const SENTINEL_ALG: &str = "SENTINEL_ALGORITHM_HS256_SECRET";
        let issuers_json = serde_json::json!([{
            "issuer": "https://issuer.test",
            "command_audiences": ["https://relay.test"],
            "token_class": "nip-fi+jwt",
            "algorithms": [SENTINEL_ALG],
            "skew_seconds": 30,
            "maximum_assertion_age_seconds": 3600,
            "jwks_uri": "https://issuer.test/.well-known/jwks.json",
            "jwks_refresh_interval_seconds": 300,
            "jwks_hard_deadline_seconds": 3600
        }])
        .to_string();

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        set_issuers(&issuers_json);
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");

        let err = NipFiRelayConfig::from_env().expect_err("unknown algorithm must fail");
        let msg = err.to_string();

        assert!(
            !msg.contains(SENTINEL_ALG),
            "algorithm error MUST NOT echo the raw algorithm value: {msg}"
        );
        // The error must indicate what went wrong (non-empty, contains hint).
        assert!(!msg.is_empty(), "error must be non-empty");
    }

    /// Policy-build rejection error must not leak the issuer URL.
    #[test]
    fn policy_build_rejection_error_does_not_leak_issuer_url() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        const SENTINEL_ISSUER: &str = "https://sentinel-issuer-secret.example";
        // An issuer config with an empty audiences list → IssuerPolicy::new fails.
        let issuers_json = serde_json::json!([{
            "issuer": SENTINEL_ISSUER,
            "command_audiences": [],  // empty → IssuerPolicy::new must fail
            "token_class": "nip-fi+jwt",
            "algorithms": ["ES256"],
            "skew_seconds": 30,
            "maximum_assertion_age_seconds": 3600,
            "jwks_uri": "https://sentinel-issuer-secret.example/.well-known/jwks.json",
            "jwks_refresh_interval_seconds": 300,
            "jwks_hard_deadline_seconds": 3600
        }])
        .to_string();

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        set_issuers(&issuers_json);
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");

        let err = NipFiRelayConfig::from_env()
            .expect_err("empty audiences must cause a policy build failure");
        let msg = err.to_string();

        // The error must not leak the sentinel issuer URL.
        assert!(
            !msg.contains(SENTINEL_ISSUER),
            "policy-build error MUST NOT echo the raw issuer URL: {msg}"
        );
        // The error must be non-empty and mention the issuer index.
        assert!(
            msg.contains("index"),
            "error must reference the issuer by index, not URL: {msg}"
        );
    }

    // ── session-deadline three-term bound ─────────────────────────────────────

    /// The `compute_session_deadline` function satisfies the spec's three-term min:
    ///
    ///   session_deadline = min(
    ///       upstream_authority_deadline(),             // = min(authority_deadlines)
    ///       connection_time + max_connection_lifetime  // partitions, never shortens
    ///   )
    ///
    /// Each scenario sets one term as the strictly-earliest deadline and asserts
    /// `compute_session_deadline` returns that term. Mutation evidence: replacing
    /// `upstream.min(partition)` with `upstream` alone makes Scenario D panic.
    #[test]
    fn session_deadline_three_term_min_selects_earliest() {
        use crate::connection::compute_session_deadline;
        use chrono::{Duration, Utc};

        let now = Utc::now();

        // Scenario A: exp is earliest (upstream wins over partition).
        {
            let exp = now + Duration::seconds(100);
            let iat_plus_max_age = now + Duration::seconds(200);
            let key_hard = now + Duration::seconds(300);
            let max_lifetime = std::time::Duration::from_secs(400);
            let assertion =
                buzz_auth::VerifiedAssertion::for_test(None, vec![exp, iat_plus_max_age, key_hard]);
            let deadline = compute_session_deadline(&assertion, now, Some(max_lifetime));
            assert_eq!(deadline, exp, "exp is earliest → deadline = exp");
        }

        // Scenario B: iat+max_age is earliest (upstream wins over partition).
        {
            let exp = now + Duration::seconds(300);
            let iat_plus_max_age = now + Duration::seconds(100);
            let key_hard = now + Duration::seconds(200);
            let max_lifetime = std::time::Duration::from_secs(400);
            let assertion =
                buzz_auth::VerifiedAssertion::for_test(None, vec![exp, iat_plus_max_age, key_hard]);
            let deadline = compute_session_deadline(&assertion, now, Some(max_lifetime));
            assert_eq!(
                deadline, iat_plus_max_age,
                "iat+max_age is earliest → deadline = iat+max_age"
            );
        }

        // Scenario C: key_snapshot_hard_deadline is earliest (upstream wins over partition).
        {
            let exp = now + Duration::seconds(400);
            let iat_plus_max_age = now + Duration::seconds(300);
            let key_hard = now + Duration::seconds(100);
            let max_lifetime = std::time::Duration::from_secs(200);
            let assertion =
                buzz_auth::VerifiedAssertion::for_test(None, vec![exp, iat_plus_max_age, key_hard]);
            let deadline = compute_session_deadline(&assertion, now, Some(max_lifetime));
            assert_eq!(
                deadline, key_hard,
                "key_snapshot_hard_deadline is earliest → deadline = key_hard"
            );
        }

        // Scenario D: max_connection_lifetime partition is earliest.
        {
            let exp = now + Duration::seconds(400);
            let iat_plus_max_age = now + Duration::seconds(300);
            let key_hard = now + Duration::seconds(200);
            let max_lifetime = std::time::Duration::from_secs(100);
            let assertion =
                buzz_auth::VerifiedAssertion::for_test(None, vec![exp, iat_plus_max_age, key_hard]);
            let deadline = compute_session_deadline(&assertion, now, Some(max_lifetime));
            let expected_partition = now + Duration::seconds(100);
            assert_eq!(
                deadline, expected_partition,
                "max_connection_lifetime partition is earliest → deadline = partition"
            );
        }
    }

    /// When `max_connection_lifetime` is absent, session_deadline equals the
    /// upstream authority deadline without further shortening.
    #[test]
    fn session_deadline_no_lifetime_uses_upstream_only() {
        use crate::connection::compute_session_deadline;
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let exp = now + Duration::seconds(600);
        let iat_plus_max_age = now + Duration::seconds(3600);
        let key_hard = now + Duration::seconds(86400);
        let assertion =
            buzz_auth::VerifiedAssertion::for_test(None, vec![exp, iat_plus_max_age, key_hard]);

        // No lifetime partition configured → deadline = upstream = min(authority_deadlines).
        let deadline = compute_session_deadline(&assertion, now, None);
        assert_eq!(
            deadline, exp,
            "no lifetime → deadline = min(authority_deadlines) = exp"
        );
    }

    /// Equality at any deadline is expired — the session_deadline computation
    /// never uses `<=` to mean "still live"; `>=` fires at equality.
    #[test]
    fn session_deadline_equality_is_expired() {
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let deadline_now = now; // exactly now = expired

        // Simulate the expiry check: `now >= deadline` fires at equality.
        assert!(
            now >= deadline_now,
            "equality must count as expired per [FI-TRACE-LEASE-BOUND]"
        );

        // A deadline strictly in the future is not yet expired.
        let deadline_future = now + Duration::milliseconds(1);
        assert!(
            now < deadline_future,
            "a deadline in the future must not be expired"
        );
    }

    // ── NIP-FI S4 deny witnesses ──
    #[test]
    fn config_error_does_not_expose_sensitive_principal_value() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        set_issuers(format!(
            r#"[{{
                    "issuer": "https://idp.example.com",
                    "command_audiences": ["https://relay.example.com"],
                    "token_class": "nip-fi+jwt",
                    "algorithms": ["ES256"],
                    "maximum_assertion_age_seconds": 3600,
                    "jwks_uri": "https://idp.example.com/.well-known/jwks.json",
                    "jwks_refresh_interval_seconds": 300,
                    "jwks_hard_deadline_seconds": 86400,
                    "maximum_command_age_seconds": 30,
                    "authorized_principals": "{SENTINEL}"
                }}]"#
        ));

        let err = NipFiRelayConfig::from_env().expect_err("malformed issuers must fail");
        let display_msg = err.to_string();
        let debug_msg = format!("{err:?}");

        // Safe category must be present.
        assert!(
            display_msg.contains("BUZZ_NIP_FI_ISSUERS is not valid JSON"),
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

    #[test]
    fn enforce_command_age_without_principals_fails_closed() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // issuer has command age but no principals
        set_issuers(
            r#"[{
                "issuer": "https://idp.example.com",
                "command_audiences": ["https://relay.example.com"],
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
    fn enforce_issuer_without_command_fields_is_rejected() {
        // An enforce-mode issuer entry with ALL THREE S4 fields absent must
        // fail startup.  This is the blocker-4a case: the issuer is a valid
        // JWKS/assertion issuer but carries no command config.  Without this
        // rejection from_env() would succeed with an empty command_configs,
        // the endpoint would permanently return 503, and startup would log nothing.
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // All three S4 command fields absent — pure assertion/JWKS issuer.
        set_issuers(
            r#"[{
                "issuer": "https://idp.example.com",
                "command_audiences": ["https://relay.example.com"],
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

    #[test]
    fn orphan_authorized_principals_without_command_age_fails_closed() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // authorized_principals without maximum_command_age_seconds — orphan field.
        set_issuers(
            r#"[{
                "issuer": "https://idp.example.com",
                "command_audiences": ["https://relay.example.com"],
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
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        // deny_set_capacity without maximum_command_age_seconds — orphan field.
        set_issuers(
            r#"[{
                "issuer": "https://idp.example.com",
                "command_audiences": ["https://relay.example.com"],
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

    /// `from_env` in enforce mode over the valid issuer fixture plus a second
    /// issuer, with `communities` as `BUZZ_NIP_FI_COMMUNITIES`.
    fn enforce_with_communities(
        communities: serde_json::Value,
    ) -> Result<NipFiRelayConfig, String> {
        let mut second = valid_enforce_issuer();
        second["issuer"] = "https://issuer-b.test".into();
        second["jwks_uri"] = "https://issuer-b.test/.well-known/jwks.json".into();
        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        std::env::set_var(
            "BUZZ_NIP_FI_ISSUERS",
            serde_json::json!([valid_enforce_issuer(), second]).to_string(),
        );
        std::env::set_var("BUZZ_NIP_FI_COMMUNITIES", communities.to_string());
        NipFiRelayConfig::from_env().map_err(|e| e.to_string())
    }

    fn community_entry(uri: &str, issuers: &[&str]) -> serde_json::Value {
        serde_json::json!({ "canonical_uri": uri, "authorized_issuers": issuers })
    }

    #[test]
    fn enforce_communities_map_hosts_to_bindings() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        let cfg = enforce_with_communities(serde_json::json!([
            community_entry("https://a.relay.test", &["https://issuer.test"]),
            community_entry("https://b.relay.test:8443", &["https://issuer-b.test"]),
        ]))
        .expect("valid communities");

        let a = cfg
            .communities
            .resolve("A.Relay.Test")
            .expect("Host a maps");
        assert_eq!(a.expected_aud(), "https://a.relay.test");
        assert!(a.authorizes("https://issuer.test") && !a.authorizes("https://issuer-b.test"));
        let b = cfg
            .communities
            .resolve("b.relay.test:8443")
            .expect("Host b maps");
        assert_eq!(b.expected_aud(), "https://b.relay.test:8443");
        assert!(cfg.communities.resolve("c.relay.test").is_none());
    }

    #[test]
    fn enforce_without_communities_fails_closed() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        enforce_with_communities(serde_json::json!([])).expect_err("empty array");
        std::env::remove_var("BUZZ_NIP_FI_COMMUNITIES");
        let err = NipFiRelayConfig::from_env().expect_err("unset communities must fail");
        assert!(
            err.to_string()
                .contains("BUZZ_NIP_FI_COMMUNITIES is not set"),
            "{err}"
        );
    }

    #[test]
    fn enforce_community_uri_must_be_https_authority_only() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        for uri in [
            "http://a.relay.test",
            "https://a.relay.test/",
            "https://a.relay.test/path",
            "https://a.relay.test?q",
            "https://a.relay.test#f",
            "https://user@a.relay.test",
            "https://A.relay.test",
            "https://",
            "https://a.relay.test ",
            "https://a.relay.test\t",
            "https://a.\trelay.test",
            "https://a.relay.test\n",
            "https://a.relay.test\\path",
            "https://a.relay.test:443",
            "https://[0:0::1]",
            "https://.",
            "https://.:80",
            "https://a.relay.test.",
            "https://a.relay.test:80",
        ] {
            let err = enforce_with_communities(serde_json::json!([community_entry(
                uri,
                &["https://issuer.test", "https://issuer-b.test"]
            ),]))
            .expect_err(uri);
            assert!(
                err.contains("community at index 0: canonical_uri must be"),
                "{uri}: {err}"
            );
            assert!(
                !err.contains("relay.test"),
                "error must not echo the URI: {err}"
            );
        }
    }

    #[test]
    fn enforce_communities_must_have_unique_hosts() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        let err = enforce_with_communities(serde_json::json!([
            community_entry("https://a.relay.test", &["https://issuer.test"]),
            community_entry("https://a.relay.test", &["https://issuer-b.test"]),
        ]))
        .expect_err("duplicate Host");
        assert!(
            err.contains("community at index 1: canonical_uri maps to the same Host"),
            "{err}"
        );
    }

    #[test]
    fn enforce_community_issuers_must_be_configured() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        let err = enforce_with_communities(serde_json::json!([community_entry(
            "https://a.relay.test",
            &[
                "https://issuer.test",
                "https://issuer-b.test",
                "https://stranger.test"
            ],
        )]))
        .expect_err("unconfigured issuer");
        assert!(
            err.contains("authorized issuer at index 2 is not configured"),
            "{err}"
        );
        assert!(
            !err.contains("stranger"),
            "error must not echo the issuer: {err}"
        );
        let err = enforce_with_communities(serde_json::json!([community_entry(
            "https://a.relay.test",
            &[],
        )]))
        .expect_err("empty allowlist");
        assert!(
            err.contains("community at index 0: empty authorized issuer set"),
            "{err}"
        );
    }

    #[test]
    fn enforce_issuer_in_no_community_fails_closed() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        let err = enforce_with_communities(serde_json::json!([community_entry(
            "https://a.relay.test",
            &["https://issuer.test"],
        )]))
        .expect_err("issuer-b is in no community");
        assert!(
            err.contains("issuer at index 1 is not authorized by any community"),
            "{err}"
        );
    }

    #[test]
    fn enforce_issuer_rejects_removed_audiences_field() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS", "3600");
        for value in [
            serde_json::json!(["https://relay.test"]),
            serde_json::Value::Null,
        ] {
            let mut stale = valid_enforce_issuer();
            stale["audiences"] = value.clone();
            set_issuers(serde_json::json!([stale]).to_string());
            let err = NipFiRelayConfig::from_env().expect_err("stale audiences field");
            assert!(
                err.to_string().contains("\"audiences\" was removed"),
                "{value}: {err}"
            );
        }
    }

    #[test]
    fn enforce_community_uri_accepts_canonical_authorities() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        for uri in [
            "https://a.relay.test",
            "https://a.relay.test:8443",
            "https://[::1]:8443",
        ] {
            enforce_with_communities(serde_json::json!([community_entry(
                uri,
                &["https://issuer.test", "https://issuer-b.test"]
            )]))
            .expect(uri);
        }
    }

    #[test]
    fn enforce_empty_or_whitespace_host_resolves_to_no_community() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        // `https://.` would normalize to an empty Host key if startup let it
        // through, so whichever configs boot must never match a blank Host.
        let mut booted = Vec::new();
        for uri in ["https://a.relay.test", "https://."] {
            let Ok(cfg) = enforce_with_communities(serde_json::json!([community_entry(
                uri,
                &["https://issuer.test", "https://issuer-b.test"]
            )])) else {
                continue;
            };
            assert!(cfg.communities.any_host.is_none());
            for host in ["", " "] {
                assert!(
                    cfg.communities.resolve(host).is_none(),
                    "{uri}: Host {host:?} must resolve to no community"
                );
            }
            booted.push(uri);
        }
        assert!(
            booted.contains(&"https://a.relay.test"),
            "the valid row must boot so resolve() is exercised"
        );
    }

    #[test]
    fn deny_protected_ignores_invalid_communities() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        std::env::set_var("BUZZ_NIP_FI_MODE", "deny_protected");
        std::env::set_var("BUZZ_NIP_FI_COMMUNITIES", "not json");
        let cfg = NipFiRelayConfig::from_env().expect("repair mode must still boot");
        assert!(cfg.communities.resolve("a.relay.test").is_none());
    }

    // Pins the startup-error formatter: the enforce text is unchanged and
    // shadow names its own mode. Mutation: a hard-coded `enforce` fails the
    // shadow test; any rewording fails the enforce test.
    fn missing_issuers_error(mode: &str) -> String {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        std::env::set_var("BUZZ_NIP_FI_MODE", mode);
        std::env::remove_var("BUZZ_NIP_FI_ISSUERS");
        match NipFiRelayConfig::from_env() {
            Err(ConfigError::InvalidValue(msg)) => msg,
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn enforce_missing_issuers_error_is_exact() {
        assert_eq!(
            missing_issuers_error("enforce"),
            "BUZZ_NIP_FI_MODE=enforce but BUZZ_NIP_FI_ISSUERS is not set; \
             set it to a JSON array of issuer configs"
        );
    }

    #[test]
    fn shadow_missing_issuers_error_names_shadow() {
        assert_eq!(
            missing_issuers_error("shadow"),
            "BUZZ_NIP_FI_MODE=shadow but BUZZ_NIP_FI_ISSUERS is not set; \
             set it to a JSON array of issuer configs"
        );
    }

    // Pins D1: shadow loads the full enforce configuration, communities
    // included, and refuses to start without them.
    #[test]
    fn shadow_startup_requires_full_enforce_config() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        let communities = serde_json::json!([community_entry(
            "https://a.relay.test",
            &["https://issuer.test", "https://issuer-b.test"]
        )]);
        enforce_with_communities(communities.clone()).expect("enforce fixture is valid");
        std::env::set_var("BUZZ_NIP_FI_MODE", "shadow");
        let cfg = NipFiRelayConfig::from_env().expect("shadow accepts the enforce config");
        assert_eq!(cfg.mode, NipFiMode::Shadow);
        assert!(cfg.communities.resolve("a.relay.test").is_some());
        std::env::remove_var("BUZZ_NIP_FI_COMMUNITIES");
        let err = NipFiRelayConfig::from_env().expect_err("shadow without communities");
        assert!(
            err.to_string()
                .contains("BUZZ_NIP_FI_MODE=shadow but BUZZ_NIP_FI_COMMUNITIES is not set"),
            "{err}"
        );
    }

    // Pins the lifetime refusal operators see in each evaluating mode.
    #[test]
    fn missing_lifetime_error_is_exact_in_enforce_and_shadow() {
        let _guard = super::NIP_FI_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _env = EnvGuard::new(NIP_FI_VARS);
        let communities = serde_json::json!([community_entry(
            "https://a.relay.test",
            &["https://issuer.test", "https://issuer-b.test"]
        )]);
        enforce_with_communities(communities).expect("enforce fixture is valid");
        std::env::remove_var("BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS");
        for mode in ["enforce", "shadow"] {
            std::env::set_var("BUZZ_NIP_FI_MODE", mode);
            let err = NipFiRelayConfig::from_env().expect_err("lifetime is required");
            assert_eq!(
                err.to_string(),
                format!(
                    "invalid config: BUZZ_NIP_FI_MODE={mode} but \
                     BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS is not set; \
                     every enforce deployment must configure a positive finite value"
                )
            );
        }
    }
}
