#[cfg(feature = "dev-app-attest-bypass")]
use crate::app_attest_policy::DevelopmentAppAttestBypass;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyConfig {
    pub id: String,
    pub key: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub health_addr: SocketAddr,
    pub public_delivery_url: url::Url,
    pub max_grant_lifetime_seconds: i64,
    pub max_installation_lifetime_seconds: i64,
    pub endpoint_quota_window_seconds: i64,
    pub endpoint_quota_max_deliveries: i64,
    pub enabled_profiles: HashSet<crate::model::AppProfile>,
    pub database_url: String,
    pub app_attest_app_id: String,
    pub app_attest_root_cert_path: PathBuf,
    #[cfg(feature = "dev-app-attest-bypass")]
    pub dev_app_attest_bypass: bool,
    /// Ordered current key first, followed by decrypt-only predecessors.
    pub grant_keys: Vec<KeyConfig>,
    /// Independent token-custody keyring. These keys MUST NOT be reused for
    /// externally presented delivery capabilities.
    pub token_keys: Vec<KeyConfig>,
    pub apns_cert_path: PathBuf,
    pub apns_topic: String,
}
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("invalid environment variable {0}")]
    Invalid(&'static str),
}
fn parse_keyring(
    e: &HashMap<String, String>,
    variable: &'static str,
) -> Result<Vec<KeyConfig>, ConfigError> {
    let value = e
        .get(variable)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::Missing(variable))?;
    let keys = value
        .split(',')
        .map(|entry| {
            let (id, encoded) = entry
                .split_once(':')
                .filter(|(id, encoded)| !id.is_empty() && !encoded.is_empty())
                .ok_or(ConfigError::Invalid(variable))?;
            let key = STANDARD
                .decode(encoded)
                .map_err(|_| ConfigError::Invalid(variable))?;
            if key.len() != 32 {
                return Err(ConfigError::Invalid(variable));
            }
            Ok(KeyConfig {
                id: id.to_owned(),
                key,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        return Err(ConfigError::Invalid(variable));
    }
    Ok(keys)
}
impl Config {
    #[cfg(feature = "dev-app-attest-bypass")]
    pub fn dev_app_attest_bypass(&self) -> Option<DevelopmentAppAttestBypass> {
        self.dev_app_attest_bypass
            .then(DevelopmentAppAttestBypass::enabled)
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_map(&std::env::vars().collect())
    }
    pub fn from_map(e: &HashMap<String, String>) -> Result<Self, ConfigError> {
        fn req<'a>(
            e: &'a HashMap<String, String>,
            k: &'static str,
        ) -> Result<&'a str, ConfigError> {
            e.get(k)
                .map(String::as_str)
                .filter(|v| !v.is_empty())
                .ok_or(ConfigError::Missing(k))
        }
        let grant_keys = parse_keyring(e, "BUZZ_PUSH_GRANT_KEYS")?;
        let token_keys = parse_keyring(e, "BUZZ_PUSH_TOKEN_KEYS")?;
        if grant_keys.iter().any(|grant| {
            token_keys
                .iter()
                .any(|token| grant.id == token.id || grant.key == token.key)
        }) {
            return Err(ConfigError::Invalid("BUZZ_PUSH_TOKEN_KEYS"));
        }
        let public_delivery_url = req(e, "BUZZ_PUSH_PUBLIC_DELIVERY_URL")?
            .parse::<url::Url>()
            .map_err(|_| ConfigError::Invalid("BUZZ_PUSH_PUBLIC_DELIVERY_URL"))?;
        if public_delivery_url.scheme() != "https"
            || public_delivery_url.host_str() != Some("push.buzz.xyz")
            || public_delivery_url.port().is_some()
            || public_delivery_url.path() != "/v1/deliveries/apns"
            || public_delivery_url.query().is_some()
            || public_delivery_url.fragment().is_some()
            || !public_delivery_url.username().is_empty()
            || public_delivery_url.password().is_some()
        {
            return Err(ConfigError::Invalid("BUZZ_PUSH_PUBLIC_DELIVERY_URL"));
        }
        let max_grant_lifetime_seconds = req(e, "BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS")?
            .parse::<i64>()
            .ok()
            .filter(|seconds| (1..=31_536_000).contains(seconds))
            .ok_or(ConfigError::Invalid("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS"))?;
        let max_installation_lifetime_seconds = e
            .get("BUZZ_PUSH_MAX_INSTALLATION_LIFETIME_SECONDS")
            .map(String::as_str)
            .unwrap_or("7776000")
            .parse::<i64>()
            .ok()
            .filter(|seconds| (1..=31_536_000).contains(seconds))
            .ok_or(ConfigError::Invalid(
                "BUZZ_PUSH_MAX_INSTALLATION_LIFETIME_SECONDS",
            ))?;
        let bounded_positive = |key: &'static str, default: i64, max: i64| {
            e.get(key)
                .map(String::as_str)
                .unwrap_or("")
                .parse::<i64>()
                .ok()
                .or((!e.contains_key(key)).then_some(default))
                .filter(|value| (1..=max).contains(value))
                .ok_or(ConfigError::Invalid(key))
        };
        let endpoint_quota_window_seconds =
            bounded_positive("BUZZ_PUSH_ENDPOINT_QUOTA_WINDOW_SECONDS", 10, 86_400)?;
        let endpoint_quota_max_deliveries =
            bounded_positive("BUZZ_PUSH_ENDPOINT_QUOTA_MAX_DELIVERIES", 10, 10_000)?;
        let enabled_profiles = req(e, "BUZZ_PUSH_ENABLED_PROFILES")?
            .split(',')
            .map(|profile| match profile {
                "buzz-ios-production" => Ok(crate::model::AppProfile::BuzzIosProduction),
                "buzz-ios-sandbox" => Ok(crate::model::AppProfile::BuzzIosSandbox),
                _ => Err(ConfigError::Invalid("BUZZ_PUSH_ENABLED_PROFILES")),
            })
            .collect::<Result<HashSet<_>, _>>()?;
        if enabled_profiles.is_empty() {
            return Err(ConfigError::Invalid("BUZZ_PUSH_ENABLED_PROFILES"));
        }
        let bind_addr = e
            .get("BUZZ_PUSH_BIND_ADDR")
            .map(String::as_str)
            .unwrap_or("0.0.0.0:8080")
            .parse::<SocketAddr>()
            .map_err(|_| ConfigError::Invalid("BUZZ_PUSH_BIND_ADDR"))?;
        let health_addr = e
            .get("BUZZ_PUSH_HEALTH_ADDR")
            .map(String::as_str)
            .unwrap_or("0.0.0.0:8081")
            .parse::<SocketAddr>()
            .map_err(|_| ConfigError::Invalid("BUZZ_PUSH_HEALTH_ADDR"))?;
        let dev_app_attest_bypass_requested =
            match e.get("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS").map(String::as_str) {
                None | Some("0") => false,
                Some("1") => true,
                Some(_) => return Err(ConfigError::Invalid("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS")),
            };
        #[cfg(not(feature = "dev-app-attest-bypass"))]
        if dev_app_attest_bypass_requested {
            return Err(ConfigError::Invalid("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS"));
        }
        #[cfg(feature = "dev-app-attest-bypass")]
        let dev_app_attest_bypass = dev_app_attest_bypass_requested;
        #[cfg(feature = "dev-app-attest-bypass")]
        if dev_app_attest_bypass
            && (!bind_addr.ip().is_loopback() || !health_addr.ip().is_loopback())
        {
            return Err(ConfigError::Invalid("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS"));
        }
        #[cfg(feature = "dev-app-attest-bypass")]
        if dev_app_attest_bypass
            && (enabled_profiles.len() != 1
                || !enabled_profiles.contains(&crate::model::AppProfile::BuzzIosSandbox))
        {
            return Err(ConfigError::Invalid("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS"));
        }
        Ok(Self {
            bind_addr,
            health_addr,
            public_delivery_url,
            max_grant_lifetime_seconds,
            max_installation_lifetime_seconds,
            endpoint_quota_window_seconds,
            endpoint_quota_max_deliveries,
            enabled_profiles,
            database_url: req(e, "DATABASE_URL")?.to_owned(),
            app_attest_app_id: req(e, "BUZZ_PUSH_APP_ATTEST_APP_ID")?.to_owned(),
            app_attest_root_cert_path: req(e, "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH")?.into(),
            #[cfg(feature = "dev-app-attest-bypass")]
            dev_app_attest_bypass,
            grant_keys,
            token_keys,
            apns_cert_path: req(e, "BUZZ_PUSH_APNS_CERT_PATH")?.into(),
            apns_topic: req(e, "BUZZ_PUSH_APNS_TOPIC")?.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "dev-app-attest-bypass")]
    use crate::app_attest_policy::AppAttestPolicy;
    #[cfg(feature = "dev-app-attest-bypass")]
    use sha2::Digest as _;

    fn base() -> HashMap<String, String> {
        HashMap::from([
            (
                "BUZZ_PUSH_GRANT_KEYS".into(),
                format!(
                    "current:{},old:{}",
                    STANDARD.encode([1; 32]),
                    STANDARD.encode([2; 32])
                ),
            ),
            (
                "BUZZ_PUSH_TOKEN_KEYS".into(),
                format!(
                    "current-token:{},old-token:{}",
                    STANDARD.encode([3; 32]),
                    STANDARD.encode([4; 32])
                ),
            ),
            (
                "BUZZ_PUSH_PUBLIC_DELIVERY_URL".into(),
                "https://push.buzz.xyz/v1/deliveries/apns".into(),
            ),
            (
                "BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS".into(),
                "2592000".into(),
            ),
            (
                "BUZZ_PUSH_ENABLED_PROFILES".into(),
                "buzz-ios-production".into(),
            ),
            (
                "DATABASE_URL".into(),
                "postgres://buzz:test@localhost/buzz".into(), // sadscan:disable np.postgres.1
            ),
            ("BUZZ_PUSH_APP_ATTEST_APP_ID".into(), "TEAM.app".into()),
            (
                "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH".into(),
                "/apple-root.pem".into(),
            ),
            ("BUZZ_PUSH_APNS_CERT_PATH".into(), "/identity.pem".into()),
            ("BUZZ_PUSH_APNS_TOPIC".into(), "app".into()),
            ("BUZZ_PUSH_BIND_ADDR".into(), "127.0.0.1:8080".into()),
            ("BUZZ_PUSH_HEALTH_ADDR".into(), "127.0.0.1:8081".into()),
        ])
    }

    #[test]
    fn certificate_path_and_topic_are_both_required() {
        let config = Config::from_map(&base()).unwrap();
        assert_eq!(config.apns_cert_path, PathBuf::from("/identity.pem"));
        assert_eq!(config.apns_topic, "app");

        for variable in ["BUZZ_PUSH_APNS_CERT_PATH", "BUZZ_PUSH_APNS_TOPIC"] {
            let mut env = base();
            env.remove(variable);
            assert!(
                matches!(Config::from_map(&env), Err(ConfigError::Missing(key)) if key == variable)
            );
        }
    }

    #[test]
    fn keyrings_preserve_current_then_predecessor_order_and_are_independent() {
        let config = Config::from_map(&base()).unwrap();
        assert_eq!(config.grant_keys[0].id, "current");
        assert_eq!(config.grant_keys[1].id, "old");
        assert_eq!(config.token_keys[0].id, "current-token");
        assert_eq!(config.token_keys[1].id, "old-token");
        assert_ne!(config.grant_keys[0].key, config.token_keys[0].key);
    }

    #[test]
    fn malformed_security_configuration_fails_startup() {
        for (key, value) in [
            (
                "BUZZ_PUSH_PUBLIC_DELIVERY_URL",
                "http://push.example/v1/deliveries/apns",
            ),
            (
                "BUZZ_PUSH_PUBLIC_DELIVERY_URL",
                "https://push.example/v1/deliveries/apns",
            ),
            ("BUZZ_PUSH_APP_ATTEST_APP_ID", ""),
            ("BUZZ_PUSH_ENABLED_PROFILES", "unknown-profile"),
            ("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS", "0"),
            ("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS", "31536001"),
            ("BUZZ_PUSH_MAX_INSTALLATION_LIFETIME_SECONDS", "0"),
        ] {
            let mut env = base();
            env.insert(key.into(), value.into());
            assert!(Config::from_map(&env).is_err(), "accepted {key}={value}");
        }
    }

    #[test]
    fn cross_keyring_id_or_material_reuse_fails_startup() {
        for token_keys in [
            format!("current:{}", STANDARD.encode([9; 32])),
            format!("other:{}", STANDARD.encode([1; 32])),
        ] {
            let mut env = base();
            env.insert("BUZZ_PUSH_TOKEN_KEYS".into(), token_keys);
            assert!(Config::from_map(&env).is_err());
        }
    }

    #[test]
    fn listener_defaults_remain_public_when_addresses_are_absent() {
        let mut env = base();
        env.remove("BUZZ_PUSH_BIND_ADDR");
        env.remove("BUZZ_PUSH_HEALTH_ADDR");

        let config = Config::from_map(&env).unwrap();
        assert_eq!(config.bind_addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.health_addr, "0.0.0.0:8081".parse().unwrap());
    }

    #[test]
    fn production_and_sandbox_profiles_parse_together_without_bypass() {
        let mut env = base();
        env.insert(
            "BUZZ_PUSH_ENABLED_PROFILES".into(),
            "buzz-ios-production,buzz-ios-sandbox".into(),
        );

        let config = Config::from_map(&env).unwrap();
        assert_eq!(config.enabled_profiles.len(), 2);
        assert!(config
            .enabled_profiles
            .contains(&crate::model::AppProfile::BuzzIosProduction));
        assert!(config
            .enabled_profiles
            .contains(&crate::model::AppProfile::BuzzIosSandbox));
    }

    #[test]
    fn dev_app_attest_bypass_flag_is_strict_and_feature_gated() {
        let absent = Config::from_map(&base()).unwrap();
        #[cfg(feature = "dev-app-attest-bypass")]
        assert!(!absent.dev_app_attest_bypass);
        #[cfg(not(feature = "dev-app-attest-bypass"))]
        let _ = absent;

        let mut disabled = base();
        disabled.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "0".into());
        let disabled = Config::from_map(&disabled).unwrap();
        #[cfg(feature = "dev-app-attest-bypass")]
        assert!(!disabled.dev_app_attest_bypass);
        #[cfg(not(feature = "dev-app-attest-bypass"))]
        let _ = disabled;

        for value in ["", "false", "true", "TRUE", "yes", " 1"] {
            let mut env = base();
            env.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), value.into());
            assert!(
                matches!(
                    Config::from_map(&env),
                    Err(ConfigError::Invalid("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS"))
                ),
                "accepted non-canonical value {value:?}"
            );
        }

        #[cfg(not(feature = "dev-app-attest-bypass"))]
        {
            let mut enabled = base();
            enabled.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "1".into());
            assert!(matches!(
                Config::from_map(&enabled),
                Err(ConfigError::Invalid("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS"))
            ));
        }
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    #[test]
    fn dev_app_attest_bypass_requires_both_loopback_listeners() {
        for (key, value) in [
            ("BUZZ_PUSH_BIND_ADDR", "0.0.0.0:8080"),
            ("BUZZ_PUSH_HEALTH_ADDR", "[::]:8081"),
        ] {
            let mut env = base();
            env.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "1".into());
            env.insert(
                "BUZZ_PUSH_ENABLED_PROFILES".into(),
                "buzz-ios-sandbox".into(),
            );
            env.insert(key.into(), value.into());
            assert!(Config::from_map(&env).is_err(), "accepted {key}={value}");
        }
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    #[test]
    fn non_loopback_bind_is_rejected_before_loopback_equivalent_is_accepted() {
        let mut non_loopback = base();
        non_loopback.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "1".into());
        non_loopback.insert(
            "BUZZ_PUSH_ENABLED_PROFILES".into(),
            "buzz-ios-sandbox".into(),
        );
        non_loopback.insert("BUZZ_PUSH_BIND_ADDR".into(), "0.0.0.0:8080".into());
        assert!(Config::from_map(&non_loopback).is_err());

        non_loopback.insert("BUZZ_PUSH_BIND_ADDR".into(), "127.0.0.1:8080".into());
        assert!(
            Config::from_map(&non_loopback)
                .unwrap()
                .dev_app_attest_bypass
        );
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    #[test]
    fn dev_app_attest_bypass_requires_sandbox_as_the_only_profile() {
        for profiles in [
            "buzz-ios-production",
            "buzz-ios-sandbox,buzz-ios-production",
        ] {
            let mut env = base();
            env.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "1".into());
            env.insert("BUZZ_PUSH_ENABLED_PROFILES".into(), profiles.into());
            assert!(
                Config::from_map(&env).is_err(),
                "accepted profiles {profiles}"
            );
        }
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    #[test]
    fn dev_app_attest_bypass_accepts_explicit_one_for_loopback_sandbox() {
        let mut env = base();
        env.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "1".into());
        env.insert(
            "BUZZ_PUSH_ENABLED_PROFILES".into(),
            "buzz-ios-sandbox".into(),
        );
        assert!(Config::from_map(&env).unwrap().dev_app_attest_bypass);
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    #[test]
    fn second_profile_is_rejected_before_sandbox_only_is_accepted() {
        let mut env = base();
        env.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "1".into());
        env.insert(
            "BUZZ_PUSH_ENABLED_PROFILES".into(),
            "buzz-ios-sandbox,buzz-ios-production".into(),
        );
        assert!(Config::from_map(&env).is_err());

        env.insert(
            "BUZZ_PUSH_ENABLED_PROFILES".into(),
            "buzz-ios-sandbox".into(),
        );
        assert!(Config::from_map(&env).unwrap().dev_app_attest_bypass);
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    #[test]
    fn dev_app_attest_bypass_selects_development_policy_from_validated_config() {
        let mut env = base();
        env.insert("BUZZ_PUSH_DEV_APP_ATTEST_BYPASS".into(), "1".into());
        env.insert(
            "BUZZ_PUSH_ENABLED_PROFILES".into(),
            "buzz-ios-sandbox".into(),
        );
        let config = Config::from_map(&env).unwrap();
        let apple = crate::app_attest::AppAttestVerifier::for_policy_test();

        let policy = AppAttestPolicy::from_config(config.dev_app_attest_bypass(), apple);

        assert!(matches!(policy, AppAttestPolicy::Development(_)));
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    #[test]
    fn bypass_unset_keeps_sentinel_on_the_apple_verifier() {
        let config = Config::from_map(&base()).unwrap();
        let policy = AppAttestPolicy::from_config(
            config.dev_app_attest_bypass(),
            crate::app_attest::AppAttestVerifier::for_policy_test(),
        );
        let mut sentinel = b"buzz-dev-app-attest-v1:".to_vec();
        sentinel.extend_from_slice(&[1; 32]);
        let key_id = sha2::Sha256::digest(&sentinel);

        assert!(policy
            .verify_attestation(
                &STANDARD.encode(sentinel),
                &STANDARD.encode(key_id),
                b"canonical enrollment transcript",
            )
            .is_err());
    }

    #[test]
    fn malformed_or_empty_keyrings_fail_startup() {
        for (variable, value) in [
            ("BUZZ_PUSH_GRANT_KEYS", ""),
            ("BUZZ_PUSH_GRANT_KEYS", "missing_separator"),
            ("BUZZ_PUSH_GRANT_KEYS", "id:bad-base64"),
            ("BUZZ_PUSH_TOKEN_KEYS", ""),
            ("BUZZ_PUSH_TOKEN_KEYS", "missing_separator"),
            ("BUZZ_PUSH_TOKEN_KEYS", "id:bad-base64"),
        ] {
            let mut env = base();
            env.insert(variable.into(), value.into());
            assert!(Config::from_map(&env).is_err());
        }
    }
}
