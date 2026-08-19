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
    /// Ordered current key first, followed by decrypt-only predecessors.
    pub grant_keys: Vec<KeyConfig>,
    /// Independent token-custody keyring. These keys MUST NOT be reused for
    /// externally presented delivery capabilities.
    pub token_keys: Vec<KeyConfig>,
    pub apns_key_path: PathBuf,
    pub apns_key_id: String,
    pub apns_team_id: String,
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
        Ok(Self {
            bind_addr: e
                .get("BUZZ_PUSH_BIND_ADDR")
                .map(String::as_str)
                .unwrap_or("0.0.0.0:8080")
                .parse()
                .map_err(|_| ConfigError::Invalid("BUZZ_PUSH_BIND_ADDR"))?,
            health_addr: e
                .get("BUZZ_PUSH_HEALTH_ADDR")
                .map(String::as_str)
                .unwrap_or("0.0.0.0:8081")
                .parse()
                .map_err(|_| ConfigError::Invalid("BUZZ_PUSH_HEALTH_ADDR"))?,
            public_delivery_url,
            max_grant_lifetime_seconds,
            max_installation_lifetime_seconds,
            endpoint_quota_window_seconds,
            endpoint_quota_max_deliveries,
            enabled_profiles,
            database_url: req(e, "DATABASE_URL")?.to_owned(),
            app_attest_app_id: req(e, "BUZZ_PUSH_APP_ATTEST_APP_ID")?.to_owned(),
            app_attest_root_cert_path: req(e, "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH")?.into(),
            grant_keys,
            token_keys,
            apns_key_path: req(e, "BUZZ_PUSH_APNS_KEY_PATH")?.into(),
            apns_key_id: req(e, "BUZZ_PUSH_APNS_KEY_ID")?.to_owned(),
            apns_team_id: req(e, "BUZZ_PUSH_APNS_TEAM_ID")?.to_owned(),
            apns_topic: req(e, "BUZZ_PUSH_APNS_TOPIC")?.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                "postgres://buzz:test@localhost/buzz".into(),
            ),
            ("BUZZ_PUSH_APP_ATTEST_APP_ID".into(), "TEAM.app".into()),
            (
                "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH".into(),
                "/apple-root.pem".into(),
            ),
            ("BUZZ_PUSH_APNS_KEY_PATH".into(), "/key.p8".into()),
            ("BUZZ_PUSH_APNS_KEY_ID".into(), "key".into()),
            ("BUZZ_PUSH_APNS_TEAM_ID".into(), "team".into()),
            ("BUZZ_PUSH_APNS_TOPIC".into(), "app".into()),
        ])
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
