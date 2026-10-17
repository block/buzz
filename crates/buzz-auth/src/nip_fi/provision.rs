//! Issuer bindings written when a community is created.
//!
//! A deployment that provisions communities under one host suffix (for
//! example `<name>.communities.buzz.xyz`) binds each new community to a fixed
//! issuer allowlist at creation time. Runtime authorization reads the stored
//! per-community rows, filtered by the same [`ProvisionBindingPolicy::is_bindable`]
//! rule, so a row outside the suffix or on a reserved host never authorizes.
//!
//! Hosts whose `https://<host>` equals a configured issuer's command audience
//! are *reserved*. Their community row may exist, but they never hold an
//! issuer binding, so a command audience can never become a community
//! assertion audience.

use std::collections::BTreeSet;

use buzz_core::tenant::normalize_host;

/// Environment variable carrying the provisioning binding JSON.
pub const PROVISION_BINDING_ENV: &str = "BUZZ_NIP_FI_PROVISION_BINDING";
/// Environment variable carrying the relay's issuer configs.
pub const ISSUERS_ENV: &str = "BUZZ_NIP_FI_ISSUERS";
/// Environment variable carrying the static enterprise community map.
pub const COMMUNITIES_ENV: &str = "BUZZ_NIP_FI_COMMUNITIES";
/// Longest issuer accepted, matching the binding table's `CHECK`.
const MAX_ISSUER_BYTES: usize = 2_048;

/// Invalid provisioning binding configuration. The message never echoes
/// configured values.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ProvisionBindingError(String);

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingJson {
    host_suffix: String,
    authorized_issuers: Vec<String>,
}

/// The fields of one `BUZZ_NIP_FI_ISSUERS` entry this module needs.
#[derive(serde::Deserialize)]
struct IssuerJson {
    issuer: String,
    #[serde(default)]
    command_audiences: Vec<String>,
}

/// The field of one `BUZZ_NIP_FI_COMMUNITIES` entry this module needs.
#[derive(serde::Deserialize)]
struct CommunityJson {
    canonical_uri: String,
}

/// Why a host cannot be bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BindRefusal {
    /// The host's audience equals a configured command audience.
    #[error("host is reserved: it is a NIP-FI command audience")]
    ReservedHost,
    /// The host is not exactly one label under the provisioning suffix.
    #[error("host is not one label under the NIP-FI provisioning host suffix")]
    OutsideSuffix,
    /// The issuer is not configured in `BUZZ_NIP_FI_ISSUERS`.
    #[error("issuer is not configured in BUZZ_NIP_FI_ISSUERS")]
    UnknownIssuer,
}

/// The authority of a canonical community URI, or `None` unless the URI is
/// exactly `https://<authority>` and the URL parser reserializes that authority
/// byte-for-byte. The round trip rejects anything the parser would repair:
/// whitespace, control characters, backslashes, uppercase, userinfo, path,
/// query, fragment, a redundant default `:443`, and non-canonical IPv6.
/// The authority must also already be in `normalize_host()` form, which
/// rejects a final trailing dot and `:80`, so the Host map key equals the
/// `aud` authority exactly and is never empty.
pub fn canonical_authority(uri: &str) -> Option<&str> {
    let authority = uri.strip_prefix("https://")?;
    let parsed = url::Url::parse(uri).ok()?;
    let host = parsed.host_str()?;
    let reserialized = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    (reserialized == authority && normalize_host(authority) == authority).then_some(authority)
}

/// Parsed provisioning binding: host suffix, issuers bound to new communities
/// under it, the configured issuer set, and the reserved command audiences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionBindingPolicy {
    host_suffix: String,
    authorized_issuers: Vec<String>,
    configured_issuers: BTreeSet<String>,
    command_audiences: BTreeSet<String>,
}

impl ProvisionBindingPolicy {
    /// Read the policy from the environment. `Ok(None)` when
    /// `BUZZ_NIP_FI_PROVISION_BINDING` is unset or empty: no community gets a
    /// binding at creation. Whatever the NIP-FI mode, the suffix must not
    /// cover a `BUZZ_NIP_FI_COMMUNITIES` host.
    pub fn from_env() -> Result<Option<Self>, ProvisionBindingError> {
        let binding = std::env::var(PROVISION_BINDING_ENV).unwrap_or_default();
        if binding.trim().is_empty() {
            return Ok(None);
        }
        let issuers = std::env::var(ISSUERS_ENV).unwrap_or_default();
        let policy = Self::parse(&binding, &issuers)?;
        policy.reject_overlap(&std::env::var(COMMUNITIES_ENV).unwrap_or_default())?;
        Ok(Some(policy))
    }

    /// Refuse a suffix that covers any host in the `BUZZ_NIP_FI_COMMUNITIES`
    /// JSON, so the static enterprise map and database-bound communities
    /// never overlap. Only each entry's `canonical_uri` is read; an unreadable
    /// declaration fails closed because the check could not be trusted.
    pub fn reject_overlap(&self, communities_json: &str) -> Result<(), ProvisionBindingError> {
        if communities_json.trim().is_empty() {
            return Ok(());
        }
        let fail =
            |message: &str| ProvisionBindingError(format!("{PROVISION_BINDING_ENV}: {message}"));
        let unreadable = || fail(&format!("{COMMUNITIES_ENV} hosts are unreadable"));
        let entries: Vec<CommunityJson> =
            serde_json::from_str(communities_json).map_err(|_| unreadable())?;
        for entry in entries {
            let host = canonical_authority(&entry.canonical_uri).ok_or_else(unreadable)?;
            if host == self.host_suffix || host.ends_with(&format!(".{}", self.host_suffix)) {
                return Err(fail(&format!(
                    "host_suffix covers a {COMMUNITIES_ENV} host"
                )));
            }
        }
        Ok(())
    }

    /// Parse the binding JSON against the issuer config JSON.
    pub fn parse(binding_json: &str, issuers_json: &str) -> Result<Self, ProvisionBindingError> {
        let fail =
            |message: &str| ProvisionBindingError(format!("{PROVISION_BINDING_ENV}: {message}"));
        let binding: BindingJson = serde_json::from_str(binding_json)
            .map_err(|e| fail(&format!("invalid JSON ({:?})", e.classify())))?;
        if issuers_json.trim().is_empty() {
            return Err(fail(&format!("requires {ISSUERS_ENV}")));
        }
        let issuers: Vec<IssuerJson> = serde_json::from_str(issuers_json).map_err(|e| {
            fail(&format!(
                "{ISSUERS_ENV} is not valid JSON ({:?})",
                e.classify()
            ))
        })?;

        let host_suffix = binding.host_suffix;
        if host_suffix.is_empty()
            || normalize_host(&host_suffix) != host_suffix
            || host_suffix.contains(':')
            || host_suffix.split('.').any(str::is_empty)
        {
            return Err(fail(
                "host_suffix must be a normalized domain name without a port",
            ));
        }
        let configured_issuers: BTreeSet<String> =
            issuers.iter().map(|entry| entry.issuer.clone()).collect();
        if binding
            .authorized_issuers
            .iter()
            .chain(&configured_issuers)
            .any(|issuer| issuer.is_empty() || issuer.len() > MAX_ISSUER_BYTES)
        {
            return Err(fail(&format!(
                "every issuer must be 1 to {MAX_ISSUER_BYTES} bytes"
            )));
        }
        let command_audiences: BTreeSet<String> = issuers
            .into_iter()
            .flat_map(|entry| entry.command_audiences)
            .collect();
        let mut authorized_issuers = binding.authorized_issuers;
        authorized_issuers.sort();
        authorized_issuers.dedup();
        if authorized_issuers.is_empty() {
            return Err(fail("authorized_issuers must not be empty"));
        }
        if authorized_issuers
            .iter()
            .any(|issuer| !configured_issuers.contains(issuer))
        {
            return Err(fail(&format!(
                "every authorized issuer must be configured in {ISSUERS_ENV}"
            )));
        }
        Ok(Self {
            host_suffix,
            authorized_issuers,
            configured_issuers,
            command_audiences,
        })
    }

    /// Issuers bound to every bindable new community.
    pub fn authorized_issuers(&self) -> &[String] {
        &self.authorized_issuers
    }

    /// The configured host suffix.
    pub fn host_suffix(&self) -> &str {
        &self.host_suffix
    }

    /// Whether `host` is reserved: its `https://<host>` is a command audience.
    pub fn is_reserved(&self, host: &str) -> bool {
        self.command_audiences.contains(&format!("https://{host}"))
    }

    /// Hosts whose `https://<host>` is a configured command audience.
    pub fn reserved_hosts(&self) -> Vec<String> {
        self.command_audiences
            .iter()
            .filter_map(|aud| aud.strip_prefix("https://"))
            .map(str::to_owned)
            .collect()
    }

    /// Whether `host` is exactly one DNS label under the suffix, with no port.
    pub fn is_suffix_host(&self, host: &str) -> bool {
        host.strip_suffix(self.host_suffix.as_str())
            .and_then(|prefix| prefix.strip_suffix('.'))
            .is_some_and(|label| !label.is_empty() && !label.contains('.') && !label.contains(':'))
    }

    /// Whether `host` may hold an issuer binding: one label under the suffix
    /// and not reserved. Creation, operator binds and the runtime read all
    /// apply this one rule.
    pub fn is_bindable(&self, host: &str) -> bool {
        self.is_suffix_host(host) && !self.is_reserved(host)
    }

    /// Issuers to bind to a newly created community at `host`: the authorized
    /// issuers for a bindable host, otherwise none.
    pub fn issuers_for_new_host(&self, host: &str) -> &[String] {
        if self.is_bindable(host) {
            &self.authorized_issuers
        } else {
            &[]
        }
    }

    /// The stored `issuers` of `host` that may authorize: none unless the host
    /// is bindable, and only issuers configured in `BUZZ_NIP_FI_ISSUERS`.
    pub fn effective_issuers(&self, host: &str, mut issuers: Vec<String>) -> Vec<String> {
        if !self.is_bindable(host) {
            return Vec::new();
        }
        issuers.retain(|issuer| self.configured_issuers.contains(issuer));
        issuers
    }

    /// Check an explicit operator bind of `issuer` to `host`.
    pub fn check_bind(&self, host: &str, issuer: &str) -> Result<(), BindRefusal> {
        if self.is_reserved(host) {
            return Err(BindRefusal::ReservedHost);
        }
        if !self.is_suffix_host(host) {
            return Err(BindRefusal::OutsideSuffix);
        }
        if !self.configured_issuers.contains(issuer) {
            return Err(BindRefusal::UnknownIssuer);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISS: &str = "https://app.builderlab.xyz/api/goose";
    const OTHER: &str = "https://other.example";

    fn issuers() -> String {
        serde_json::json!([
            {"issuer": ISS, "command_audiences": ["https://communities.buzz.xyz"], "token_class": "nip-fi+jwt"},
            {"issuer": OTHER, "command_audiences": ["https://relay.other.example"]}
        ])
        .to_string()
    }

    fn policy() -> ProvisionBindingPolicy {
        ProvisionBindingPolicy::parse(
            &serde_json::json!({"host_suffix": "communities.buzz.xyz", "authorized_issuers": [ISS]})
                .to_string(),
            &issuers(),
        )
        .expect("valid policy")
    }

    #[test]
    fn provision_binding_binds_only_one_label_suffix_hosts() {
        let policy = policy();
        assert_eq!(
            policy.issuers_for_new_host("acme.communities.buzz.xyz"),
            [ISS]
        );
        for host in [
            "communities.buzz.xyz",
            "a.b.communities.buzz.xyz",
            "acme.communities.buzz.xyz:8443",
            "acmecommunities.buzz.xyz",
            ".communities.buzz.xyz",
            "acme.example",
        ] {
            assert!(policy.issuers_for_new_host(host).is_empty(), "{host}");
        }
    }

    #[test]
    fn provision_binding_reserves_command_audience_hosts() {
        let policy = policy();
        assert!(policy.is_reserved("communities.buzz.xyz"));
        assert!(policy.is_reserved("relay.other.example"));
        assert!(!policy.is_reserved("acme.communities.buzz.xyz"));
        assert_eq!(
            policy.reserved_hosts(),
            ["communities.buzz.xyz", "relay.other.example"]
        );
        assert_eq!(
            policy.check_bind("communities.buzz.xyz", ISS),
            Err(BindRefusal::ReservedHost)
        );
        assert_eq!(
            policy.check_bind("acme.communities.buzz.xyz", "https://unknown.example"),
            Err(BindRefusal::UnknownIssuer)
        );
        assert_eq!(
            policy.check_bind("acme.communities.buzz.xyz", OTHER),
            Ok(())
        );
        for host in ["a.b.communities.buzz.xyz", "acme.example"] {
            assert_eq!(
                policy.check_bind(host, ISS),
                Err(BindRefusal::OutsideSuffix),
                "{host}"
            );
        }
    }

    #[test]
    fn provision_binding_effective_issuers_need_a_bindable_host_and_configured_issuer() {
        let policy = policy();
        let stored = vec![ISS.to_string(), "https://unconfigured.example".to_string()];
        assert_eq!(
            policy.effective_issuers("acme.communities.buzz.xyz", stored.clone()),
            [ISS]
        );
        for host in [
            "communities.buzz.xyz",
            "a.b.communities.buzz.xyz",
            "acme.example",
        ] {
            assert!(
                policy.effective_issuers(host, stored.clone()).is_empty(),
                "{host}"
            );
        }
    }

    #[test]
    fn provision_binding_rejects_a_suffix_covering_an_enterprise_host() {
        let policy = policy();
        let communities = |uri: &str| {
            serde_json::json!([{"canonical_uri": uri, "authorized_issuers": [ISS]}]).to_string()
        };
        assert!(policy.reject_overlap("").is_ok());
        assert!(policy
            .reject_overlap(&communities("https://acme.example"))
            .is_ok());
        for json in [
            communities("https://communities.buzz.xyz"),
            communities("https://a.b.communities.buzz.xyz"),
            communities("https://Acme.example"),
            "not json".to_string(),
        ] {
            assert!(policy.reject_overlap(&json).is_err(), "{json}");
        }
    }

    #[test]
    fn provision_binding_rejects_invalid_config() {
        let binding = |suffix: &str, issuers: &[&str]| {
            serde_json::json!({"host_suffix": suffix, "authorized_issuers": issuers}).to_string()
        };
        for (binding, issuers) in [
            (binding("communities.buzz.xyz", &[]), issuers()),
            (
                binding("communities.buzz.xyz", &["https://unknown.example"]),
                issuers(),
            ),
            (binding("Communities.buzz.xyz", &[ISS]), issuers()),
            (binding("communities.buzz.xyz:443", &[ISS]), issuers()),
            (binding("communities..xyz", &[ISS]), issuers()),
            (binding("", &[ISS]), issuers()),
            (binding("communities.buzz.xyz", &[ISS]), String::new()),
            (binding("communities.buzz.xyz", &[""]), issuers()),
            (
                binding("communities.buzz.xyz", &[ISS]),
                serde_json::json!([{"issuer": ISS}, {"issuer": "x".repeat(2049)}]).to_string(),
            ),
            ("{\"host_suffix\": \"x\"}".to_string(), issuers()),
        ] {
            assert!(
                ProvisionBindingPolicy::parse(&binding, &issuers).is_err(),
                "{binding}"
            );
        }
    }
}
