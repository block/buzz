use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApnsEnvironment {
    Production,
    Sandbox,
}

/// The single server-owned Apple application and APNs connection identity.
#[derive(Debug, Clone)]
pub struct ApplicationConfig {
    pub app_attest_app_id: String,
    /// Exact Apple attestation environment accepted for enrollment.
    pub app_attest_environment: AppAttestEnvironment,
    pub apns_cert_path: PathBuf,
    pub apns_topic: String,
    pub apns_environment: ApnsEnvironment,
}

/// The Android application and its Firebase Cloud Messaging identity. All four
/// settings are supplied together or not at all; without them the gateway
/// accepts no Android enrollment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidConfig {
    /// Exact Android application package the attestation must name.
    pub package_name: String,
    /// Accepted SHA-256 digests of the app's signing certificate.
    pub signing_cert_digests: Vec<[u8; 32]>,
    /// Firebase project that owns the app's FCM registration tokens.
    pub fcm_project_id: String,
    /// Mounted service account key JSON the gateway uses to call FCM HTTP v1.
    pub fcm_service_account_path: PathBuf,
    /// Verified boot keys (SHA-256 digests) accepted when the hardware reports
    /// a SelfSigned boot state. Empty, the default, accepts stock Android only.
    pub boot_key_digests: Vec<[u8; 32]>,
    /// PEM bundle of attestation roots that replaces Google's built-in
    /// defaults, for a reviewed root rollout ahead of a release.
    pub attestation_roots_path: Option<PathBuf>,
    /// How long the attestation status list may go without a successful
    /// refresh before Android enrollment fails closed.
    pub revocation_grace_seconds: i64,
}

/// Apple App Attest environment, independent of the APNs transport environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAttestEnvironment {
    /// Distributed applications, also the default for personal development builds.
    Production,
    /// Development-signed applications in an explicitly opted-in gateway build.
    #[cfg(feature = "personal-dev-app-attest")]
    Development,
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
    pub max_grant_lifetime_seconds: i64,
    pub max_installation_lifetime_seconds: i64,
    pub endpoint_quota_window_seconds: i64,
    pub endpoint_quota_max_deliveries: i64,
    /// Window of the per-attested-key limit on signed installation calls.
    pub signed_call_window_seconds: i64,
    /// Signed installation calls one attested key may make per window.
    pub signed_call_max_calls: i64,
    /// Server-owned application identity and APNs transport.
    pub application: ApplicationConfig,
    /// Android identity and FCM credentials, when this gateway serves Android.
    pub android: Option<AndroidConfig>,
    pub database_url: String,
    pub app_attest_root_cert_path: PathBuf,
    /// Ordered current key first, followed by decrypt-only predecessors.
    pub grant_keys: Vec<KeyConfig>,
    /// Independent token-custody keyring. These keys MUST NOT be reused for
    /// externally presented delivery capabilities.
    pub token_keys: Vec<KeyConfig>,
}

// Registered NIP-PL v1 App Attest audiences are protocol constants, not routing URLs.
/// Fixed NIP-PL v1 App Attest audience for installation enrollment.
pub const ENROLL_AUDIENCE: &str = "https://push.buzz.xyz/v1/installations";
/// Fixed NIP-PL v1 App Attest audience for creating a delegation.
pub const DELEGATE_AUDIENCE: &str = "https://push.buzz.xyz/v1/delegations";
/// Fixed NIP-PL v1 App Attest audience for rotating an installation endpoint.
pub const ROTATE_ENDPOINT_AUDIENCE: &str = "https://push.buzz.xyz/v1/installations/endpoint";
/// Fixed NIP-PL v1 App Attest audience for revoking a delegation.
pub const REVOKE_DELEGATION_AUDIENCE: &str = "https://push.buzz.xyz/v1/delegations/revoke";
/// Fixed NIP-PL v1 App Attest audience for revoking an installation.
pub const REVOKE_INSTALLATION_AUDIENCE: &str = "https://push.buzz.xyz/v1/installations/revoke";

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

fn parse_application(e: &HashMap<String, String>) -> Result<ApplicationConfig, ConfigError> {
    let app_id_key = "BUZZ_PUSH_APP_ATTEST_APP_ID";
    let cert_key = "BUZZ_PUSH_APNS_CERT_PATH";
    let topic_key = "BUZZ_PUSH_APNS_TOPIC";
    let environment_key = "BUZZ_PUSH_APNS_ENVIRONMENT";
    let required = |key: &'static str| {
        e.get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::Missing(key))
    };
    let app_attest_app_id = required(app_id_key)?.to_owned();
    let app_attest_environment = match e
        .get("BUZZ_PUSH_APP_ATTEST_ENVIRONMENT")
        .map(String::as_str)
    {
        None | Some("production") => AppAttestEnvironment::Production,
        #[cfg(feature = "personal-dev-app-attest")]
        Some("development") => AppAttestEnvironment::Development,
        Some(_) => return Err(ConfigError::Invalid("BUZZ_PUSH_APP_ATTEST_ENVIRONMENT")),
    };
    let apns_topic = required(topic_key)?.to_owned();
    let apns_cert_path = PathBuf::from(required(cert_key)?);
    let apns_environment = match e.get(environment_key).map(String::as_str) {
        None | Some("production") => ApnsEnvironment::Production,
        Some("sandbox") => ApnsEnvironment::Sandbox,
        Some(_) => return Err(ConfigError::Invalid(environment_key)),
    };
    Ok(ApplicationConfig {
        app_attest_app_id,
        app_attest_environment,
        apns_cert_path,
        apns_topic,
        apns_environment,
    })
}

const ANDROID_SETTINGS: [&str; 4] = [
    "BUZZ_PUSH_ANDROID_PACKAGE_NAME",
    "BUZZ_PUSH_ANDROID_SIGNING_CERT_DIGESTS",
    "BUZZ_PUSH_FCM_PROJECT_ID",
    "BUZZ_PUSH_FCM_SERVICE_ACCOUNT_PATH",
];

const ANDROID_OPTIONAL_SETTINGS: [&str; 3] = [
    "BUZZ_PUSH_ANDROID_BOOT_KEY_DIGESTS",
    "BUZZ_PUSH_ANDROID_ATTESTATION_ROOTS_PATH",
    "BUZZ_PUSH_ANDROID_REVOCATION_GRACE_SECONDS",
];

/// A comma-separated list of SHA-256 digests in hex. Empty means none.
fn parse_digests(value: &str, key: &'static str) -> Result<Vec<[u8; 32]>, ConfigError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|digest| {
            hex::decode(digest)
                .ok()
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or(ConfigError::Invalid(key))
        })
        .collect()
}

fn valid_android_package_name(name: &str) -> bool {
    let segments: Vec<&str> = name.split('.').collect();
    segments.len() >= 2
        && segments.iter().all(|segment| {
            segment
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

fn parse_android(e: &HashMap<String, String>) -> Result<Option<AndroidConfig>, ConfigError> {
    // A partial configuration is a mistake, not a request to disable Android.
    if ANDROID_SETTINGS
        .iter()
        .chain(&ANDROID_OPTIONAL_SETTINGS)
        .all(|key| !e.contains_key(*key))
    {
        return Ok(None);
    }
    let required = |key: &'static str| {
        e.get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .ok_or(ConfigError::Missing(key))
    };
    let package_key = ANDROID_SETTINGS[0];
    let digests_key = ANDROID_SETTINGS[1];
    let package_name = required(package_key)?;
    if !valid_android_package_name(package_name) {
        return Err(ConfigError::Invalid(package_key));
    }
    let signing_cert_digests = parse_digests(required(digests_key)?, digests_key)?;
    let boot_key_key = ANDROID_OPTIONAL_SETTINGS[0];
    let boot_key_digests = parse_digests(
        e.get(boot_key_key).map(String::as_str).unwrap_or(""),
        boot_key_key,
    )?;
    let grace_key = ANDROID_OPTIONAL_SETTINGS[2];
    let revocation_grace_seconds = match e.get(grace_key) {
        None => 604_800,
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|seconds| (3600..=31_536_000).contains(seconds))
            .ok_or(ConfigError::Invalid(grace_key))?,
    };
    Ok(Some(AndroidConfig {
        package_name: package_name.to_owned(),
        signing_cert_digests,
        fcm_project_id: required(ANDROID_SETTINGS[2])?.to_owned(),
        fcm_service_account_path: PathBuf::from(required(ANDROID_SETTINGS[3])?),
        boot_key_digests,
        attestation_roots_path: e
            .get(ANDROID_OPTIONAL_SETTINGS[1])
            .filter(|path| !path.is_empty())
            .map(PathBuf::from),
        revocation_grace_seconds,
    }))
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
        let signed_call_window_seconds =
            bounded_positive("BUZZ_PUSH_SIGNED_CALL_WINDOW_SECONDS", 60, 86_400)?;
        let signed_call_max_calls =
            bounded_positive("BUZZ_PUSH_SIGNED_CALL_MAX_CALLS", 30, 10_000)?;
        let application = parse_application(e)?;
        let android = parse_android(e)?;
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
            max_grant_lifetime_seconds,
            max_installation_lifetime_seconds,
            endpoint_quota_window_seconds,
            endpoint_quota_max_deliveries,
            signed_call_window_seconds,
            signed_call_max_calls,
            application,
            android,
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
    #[test]
    fn attestation_environment_is_explicit_and_build_gated() {
        let mut env = base();
        assert_eq!(
            Config::from_map(&env)
                .unwrap()
                .application
                .app_attest_environment,
            AppAttestEnvironment::Production
        );
        for value in ["", "sandbox", "Production", "unknown"] {
            env.insert("BUZZ_PUSH_APP_ATTEST_ENVIRONMENT".into(), value.into());
            assert!(Config::from_map(&env).is_err());
        }
        env.insert(
            "BUZZ_PUSH_APP_ATTEST_ENVIRONMENT".into(),
            "production".into(),
        );
        assert_eq!(
            Config::from_map(&env)
                .unwrap()
                .application
                .app_attest_environment,
            AppAttestEnvironment::Production
        );
        env.insert(
            "BUZZ_PUSH_APP_ATTEST_ENVIRONMENT".into(),
            "development".into(),
        );
        #[cfg(feature = "personal-dev-app-attest")]
        assert_eq!(
            Config::from_map(&env)
                .unwrap()
                .application
                .app_attest_environment,
            AppAttestEnvironment::Development
        );
        #[cfg(not(feature = "personal-dev-app-attest"))]
        assert!(Config::from_map(&env).is_err());
    }

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
                "BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS".into(),
                "2592000".into(),
            ),
            (
                "DATABASE_URL".into(),
                "postgres://buzz:test@localhost/buzz".into(), // sadscan:disable np.postgres.1
            ),
            (
                "BUZZ_PUSH_APP_ATTEST_APP_ID".into(),
                "TEAM.com.example.buzz".into(),
            ),
            (
                "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH".into(),
                "/apple-root.pem".into(),
            ),
            (
                "BUZZ_PUSH_APNS_CERT_PATH".into(),
                "/apns-identity.pem".into(),
            ),
            ("BUZZ_PUSH_APNS_TOPIC".into(), "com.example.buzz".into()),
            ("BUZZ_PUSH_APNS_ENVIRONMENT".into(), "production".into()),
            ("BUZZ_PUSH_BIND_ADDR".into(), "127.0.0.1:8080".into()),
            ("BUZZ_PUSH_HEALTH_ADDR".into(), "127.0.0.1:8081".into()),
        ])
    }

    #[test]
    fn application_requires_server_owned_identity_and_certificate() {
        let config = Config::from_map(&base()).unwrap();
        assert_eq!(
            config.application.apns_cert_path,
            PathBuf::from("/apns-identity.pem")
        );
        assert_eq!(config.application.apns_topic, "com.example.buzz");

        for variable in [
            "BUZZ_PUSH_APNS_CERT_PATH",
            "BUZZ_PUSH_APNS_TOPIC",
            "BUZZ_PUSH_APP_ATTEST_APP_ID",
        ] {
            let mut env = base();
            env.remove(variable);
            assert!(
                matches!(Config::from_map(&env), Err(ConfigError::Missing(key)) if key == variable)
            );
        }
    }

    #[test]
    fn startup_needs_no_origin_and_transcript_audiences_stay_registered() {
        assert!(!base().contains_key("BUZZ_PUSH_GATEWAY_ORIGIN"));
        Config::from_map(&base()).unwrap();
        assert_eq!(ENROLL_AUDIENCE, "https://push.buzz.xyz/v1/installations");
        assert_eq!(DELEGATE_AUDIENCE, "https://push.buzz.xyz/v1/delegations");
        assert_eq!(
            ROTATE_ENDPOINT_AUDIENCE,
            "https://push.buzz.xyz/v1/installations/endpoint"
        );
        assert_eq!(
            REVOKE_DELEGATION_AUDIENCE,
            "https://push.buzz.xyz/v1/delegations/revoke"
        );
        assert_eq!(
            REVOKE_INSTALLATION_AUDIENCE,
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
            ("BUZZ_PUSH_APP_ATTEST_APP_ID", ""),
            ("BUZZ_PUSH_APNS_ENVIRONMENT", "staging"),
            ("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS", "0"),
            ("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS", "31536001"),
            ("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS", "2.592e+06"),
            ("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS", "2592000.5"),
            ("BUZZ_PUSH_MAX_INSTALLATION_LIFETIME_SECONDS", "0"),
        ] {
            let mut env = base();
            env.insert(key.into(), value.into());
            assert!(Config::from_map(&env).is_err(), "accepted {key}={value}");
        }
    }

    // The chart test supplies actual Helm-rendered environment values. Keep this
    // separate from ordinary unit tests, which do not require Helm or fixtures.
    #[test]
    #[ignore = "run deploy/charts/buzz-push-gateway/tests/grant-lifetime.sh"]
    fn helm_rendered_grant_lifetimes() {
        let path = std::env::var("BUZZ_TEST_HELM_ENV_FILE").unwrap();
        let cases: Vec<(String, i64, HashMap<String, String>)> =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(cases.len(), 9);
        for (label, expected, rendered) in cases {
            let mut env = base();
            env.extend(rendered);
            let config = Config::from_map(&env).unwrap_or_else(|error| panic!("{label}: {error}"));
            assert_eq!(config.max_grant_lifetime_seconds, expected, "{label}");
            assert_eq!(
                env["BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS"],
                expected.to_string(),
                "{label} must render decimal integer text"
            );
        }
    }

    fn android_env() -> HashMap<String, String> {
        let mut env = base();
        env.extend([
            (
                "BUZZ_PUSH_ANDROID_PACKAGE_NAME".into(),
                "com.example.buzz".into(),
            ),
            (
                "BUZZ_PUSH_ANDROID_SIGNING_CERT_DIGESTS".into(),
                format!("{},{}", "ab".repeat(32), "CD".repeat(32)),
            ),
            ("BUZZ_PUSH_FCM_PROJECT_ID".into(), "example-project".into()),
            (
                "BUZZ_PUSH_FCM_SERVICE_ACCOUNT_PATH".into(),
                "/fcm/service-account.json".into(),
            ),
        ]);
        env
    }

    #[test]
    fn android_is_disabled_unless_fully_configured() {
        assert_eq!(Config::from_map(&base()).unwrap().android, None);
        let android = Config::from_map(&android_env()).unwrap().android.unwrap();
        assert_eq!(android.package_name, "com.example.buzz");
        assert_eq!(android.signing_cert_digests, vec![[0xab; 32], [0xcd; 32]]);
        assert_eq!(android.fcm_project_id, "example-project");
        assert!(android.boot_key_digests.is_empty());
        assert_eq!(android.attestation_roots_path, None);
        assert_eq!(android.revocation_grace_seconds, 604_800);
        let mut env = android_env();
        env.extend([
            (
                "BUZZ_PUSH_ANDROID_BOOT_KEY_DIGESTS".to_owned(),
                format!("{},{}", "01".repeat(32), "02".repeat(32)),
            ),
            (
                "BUZZ_PUSH_ANDROID_ATTESTATION_ROOTS_PATH".to_owned(),
                "/roots.pem".to_owned(),
            ),
            (
                "BUZZ_PUSH_ANDROID_REVOCATION_GRACE_SECONDS".to_owned(),
                "7200".to_owned(),
            ),
        ]);
        let android = Config::from_map(&env).unwrap().android.unwrap();
        assert_eq!(android.boot_key_digests, vec![[1; 32], [2; 32]]);
        assert_eq!(android.attestation_roots_path, Some("/roots.pem".into()));
        assert_eq!(android.revocation_grace_seconds, 7200);
        // An empty boot-key list is the default, as Helm renders it.
        env.insert("BUZZ_PUSH_ANDROID_BOOT_KEY_DIGESTS".into(), String::new());
        assert!(Config::from_map(&env)
            .unwrap()
            .android
            .unwrap()
            .boot_key_digests
            .is_empty());
        // An Android option without the required settings is a mistake.
        let mut env = base();
        env.insert("BUZZ_PUSH_ANDROID_BOOT_KEY_DIGESTS".into(), String::new());
        assert!(Config::from_map(&env).is_err());
        for variable in ANDROID_SETTINGS {
            let mut env = android_env();
            env.remove(variable);
            assert!(
                matches!(Config::from_map(&env), Err(ConfigError::Missing(key)) if key == variable),
                "accepted partial Android configuration without {variable}"
            );
        }
    }

    #[test]
    fn malformed_android_configuration_fails_startup() {
        for (key, value) in [
            ("BUZZ_PUSH_ANDROID_PACKAGE_NAME", "buzz"),
            ("BUZZ_PUSH_ANDROID_PACKAGE_NAME", "com..buzz"),
            ("BUZZ_PUSH_ANDROID_PACKAGE_NAME", "com.1buzz"),
            ("BUZZ_PUSH_ANDROID_SIGNING_CERT_DIGESTS", ""),
            ("BUZZ_PUSH_ANDROID_SIGNING_CERT_DIGESTS", "abcd"),
            ("BUZZ_PUSH_ANDROID_SIGNING_CERT_DIGESTS", "zz"),
            ("BUZZ_PUSH_FCM_PROJECT_ID", ""),
            ("BUZZ_PUSH_ANDROID_BOOT_KEY_DIGESTS", "abcd"),
            ("BUZZ_PUSH_ANDROID_BOOT_KEY_DIGESTS", "zz"),
            ("BUZZ_PUSH_ANDROID_REVOCATION_GRACE_SECONDS", "0"),
            ("BUZZ_PUSH_ANDROID_REVOCATION_GRACE_SECONDS", "soon"),
            ("BUZZ_PUSH_SIGNED_CALL_MAX_CALLS", "0"),
            ("BUZZ_PUSH_SIGNED_CALL_WINDOW_SECONDS", "x"),
        ] {
            let mut env = android_env();
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
