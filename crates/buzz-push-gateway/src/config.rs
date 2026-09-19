use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApnsEnvironment {
    Production,
    Sandbox,
}

#[derive(Debug, Clone)]
pub struct AppProfileConfig {
    pub app_attest_app_id: String,
    pub apns_cert_path: PathBuf,
    pub apns_topic: String,
    pub apns_environment: ApnsEnvironment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyConfig {
    pub id: String,
    pub key: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub health_addr: SocketAddr,
    /// External gateway origin, delivery URL, and registered transcript audiences.
    pub gateway_urls: GatewayUrls,
    pub max_grant_lifetime_seconds: i64,
    pub max_installation_lifetime_seconds: i64,
    pub endpoint_quota_window_seconds: i64,
    pub endpoint_quota_max_deliveries: i64,
    /// Server-owned dogfood application identity and APNs transport.
    pub profile: AppProfileConfig,
    pub database_url: String,
    pub app_attest_root_cert_path: PathBuf,
    /// Ordered current key first, followed by decrypt-only predecessors.
    pub grant_keys: Vec<KeyConfig>,
    /// Independent token-custody keyring. These keys MUST NOT be reused for
    /// externally presented delivery capabilities.
    pub token_keys: Vec<KeyConfig>,
}

/// Gateway transport URLs and registered NIP-PL v1 transcript audiences.
#[derive(Debug, Clone)]
pub struct GatewayUrls {
    /// External HTTPS origin serving the gateway.
    pub origin: url::Url,
    /// Exact NIP-98 delivery endpoint used by relays.
    pub delivery: url::Url,
    /// App Attest audience for installation enrollment.
    pub enroll_audience: String,
    /// App Attest audience for relay delegation.
    pub delegate_audience: String,
    /// App Attest audience for endpoint rotation.
    pub rotate_endpoint_audience: String,
    /// App Attest audience for delegation revocation.
    pub revoke_delegation_audience: String,
    /// App Attest audience for installation revocation.
    pub revoke_installation_audience: String,
}

impl GatewayUrls {
    pub(crate) fn from_origin(origin: url::Url) -> Result<Self, ConfigError> {
        let derive = |path: &str| {
            origin
                .join(path)
                .map_err(|_| ConfigError::Invalid("BUZZ_PUSH_GATEWAY_ORIGIN"))
        };
        let delivery = derive("v1/deliveries/apns")?;
        // NIP-PL v1 registers these exact audience strings. The configurable
        // origin controls transport only; changing transcript bytes requires
        // a separately versioned protocol profile.
        let enroll_audience = "https://push.buzz.xyz/v1/installations".to_owned();
        let delegate_audience = "https://push.buzz.xyz/v1/delegations".to_owned();
        let rotate_endpoint_audience = "https://push.buzz.xyz/v1/installations/endpoint".to_owned();
        let revoke_delegation_audience = "https://push.buzz.xyz/v1/delegations/revoke".to_owned();
        let revoke_installation_audience =
            "https://push.buzz.xyz/v1/installations/revoke".to_owned();
        Ok(Self {
            origin,
            delivery,
            enroll_audience,
            delegate_audience,
            rotate_endpoint_audience,
            revoke_delegation_audience,
            revoke_installation_audience,
        })
    }
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

fn parse_profile(e: &HashMap<String, String>) -> Result<AppProfileConfig, ConfigError> {
    let app_id_key = "BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID";
    let cert_key = "BUZZ_PUSH_DOGFOOD_APNS_CERT_PATH";
    let topic_key = "BUZZ_PUSH_DOGFOOD_APNS_TOPIC";
    let environment_key = "BUZZ_PUSH_DOGFOOD_APNS_ENVIRONMENT";
    let required = |key: &'static str| {
        e.get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::Missing(key))
    };
    let app_attest_app_id = required(app_id_key)?.to_owned();
    let apns_topic = required(topic_key)?.to_owned();
    let apns_cert_path = PathBuf::from(required(cert_key)?);
    let apns_environment = match e.get(environment_key).map(String::as_str) {
        None | Some("production") => ApnsEnvironment::Production,
        Some("sandbox") => ApnsEnvironment::Sandbox,
        Some(_) => return Err(ConfigError::Invalid(environment_key)),
    };
    Ok(AppProfileConfig {
        app_attest_app_id,
        apns_cert_path,
        apns_topic,
        apns_environment,
    })
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
        let gateway_origin = req(e, "BUZZ_PUSH_GATEWAY_ORIGIN")?
            .parse::<url::Url>()
            .map_err(|_| ConfigError::Invalid("BUZZ_PUSH_GATEWAY_ORIGIN"))?;
        if gateway_origin.scheme() != "https"
            || gateway_origin.host().is_none()
            || gateway_origin.port().is_some()
            || gateway_origin.path() != "/"
            || gateway_origin.query().is_some()
            || gateway_origin.fragment().is_some()
            || !gateway_origin.username().is_empty()
            || gateway_origin.password().is_some()
        {
            return Err(ConfigError::Invalid("BUZZ_PUSH_GATEWAY_ORIGIN"));
        }
        let gateway_urls = GatewayUrls::from_origin(gateway_origin)?;
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
        let profile = parse_profile(e)?;
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
        Ok(Self {
            bind_addr,
            health_addr,
            gateway_urls,
            max_grant_lifetime_seconds,
            max_installation_lifetime_seconds,
            endpoint_quota_window_seconds,
            endpoint_quota_max_deliveries,
            profile,
            database_url: req(e, "DATABASE_URL")?.to_owned(),
            app_attest_root_cert_path: req(e, "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH")?.into(),
            grant_keys,
            token_keys,
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
                "BUZZ_PUSH_GATEWAY_ORIGIN".into(),
                "https://push.example".into(),
            ),
            (
                "BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS".into(),
                "2592000".into(),
            ),
            (
                "DATABASE_URL".into(),
                "postgres://buzz:test@localhost/buzz".into(), // sadscan:disable np.postgres.1
            ),
            (
                "BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID".into(),
                "TEAM.xyz.block.buzz.dogfood.mobile".into(),
            ),
            (
                "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH".into(),
                "/apple-root.pem".into(),
            ),
            (
                "BUZZ_PUSH_DOGFOOD_APNS_CERT_PATH".into(),
                "/dogfood-identity.pem".into(),
            ),
            (
                "BUZZ_PUSH_DOGFOOD_APNS_TOPIC".into(),
                "xyz.block.buzz.dogfood.mobile".into(),
            ),
            (
                "BUZZ_PUSH_DOGFOOD_APNS_ENVIRONMENT".into(),
                "production".into(),
            ),
            ("BUZZ_PUSH_BIND_ADDR".into(), "127.0.0.1:8080".into()),
            ("BUZZ_PUSH_HEALTH_ADDR".into(), "127.0.0.1:8081".into()),
        ])
    }

    #[test]
    fn dogfood_profile_requires_server_owned_identity_and_certificate() {
        let config = Config::from_map(&base()).unwrap();
        assert_eq!(
            config.profile.apns_cert_path,
            PathBuf::from("/dogfood-identity.pem")
        );
        assert_eq!(config.profile.apns_topic, "xyz.block.buzz.dogfood.mobile");

        for variable in [
            "BUZZ_PUSH_DOGFOOD_APNS_CERT_PATH",
            "BUZZ_PUSH_DOGFOOD_APNS_TOPIC",
            "BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID",
        ] {
            let mut env = base();
            env.remove(variable);
            assert!(
                matches!(Config::from_map(&env), Err(ConfigError::Missing(key)) if key == variable)
            );
        }
    }

    #[test]
    fn gateway_transport_uses_configured_origin_and_transcript_audiences_stay_registered() {
        let config = Config::from_map(&base()).unwrap();
        assert_eq!(config.gateway_urls.origin.as_str(), "https://push.example/");
        assert_eq!(
            config.gateway_urls.delivery.as_str(),
            "https://push.example/v1/deliveries/apns"
        );
        assert_eq!(
            config.gateway_urls.enroll_audience,
            "https://push.buzz.xyz/v1/installations"
        );
        assert_eq!(
            config.gateway_urls.delegate_audience,
            "https://push.buzz.xyz/v1/delegations"
        );
        assert_eq!(
            config.gateway_urls.rotate_endpoint_audience,
            "https://push.buzz.xyz/v1/installations/endpoint"
        );
        assert_eq!(
            config.gateway_urls.revoke_delegation_audience,
            "https://push.buzz.xyz/v1/delegations/revoke"
        );
        assert_eq!(
            config.gateway_urls.revoke_installation_audience,
            "https://push.buzz.xyz/v1/installations/revoke"
        );
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
            ("BUZZ_PUSH_GATEWAY_ORIGIN", "http://push.example"),
            ("BUZZ_PUSH_GATEWAY_ORIGIN", "https://push.example/path"),
            ("BUZZ_PUSH_GATEWAY_ORIGIN", "https://push.example?token=x"),
            ("BUZZ_PUSH_GATEWAY_ORIGIN", "https://user@push.example"),
            ("BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID", ""),
            ("BUZZ_PUSH_DOGFOOD_APNS_ENVIRONMENT", "staging"),
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
