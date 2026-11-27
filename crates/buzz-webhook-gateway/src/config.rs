//! Process configuration, read from `BUZZ_WEBHOOK_*` environment variables.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use nostr::{Keys, PublicKey};
use std::{collections::HashMap, net::SocketAddr};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("invalid value for environment variable {0}")]
    Invalid(&'static str),
}

/// One symmetric key used to seal provider secrets at rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyConfig {
    pub id: String,
    pub key: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Public listener for provider webhook deliveries.
    pub listen_addr: SocketAddr,
    /// Private listener for the relay-facing management API, health, and metrics.
    pub private_addr: SocketAddr,
    pub database_url: String,
    /// Nostr key the gateway signs webhook-call events with.
    pub signing_keys: Keys,
    /// Keys that seal provider secrets at rest; the first entry seals new secrets.
    pub secret_keyring: Vec<KeyConfig>,
    /// Relay pubkeys allowed to call the management API.
    pub relay_pubkeys: Vec<PublicKey>,
    /// `https` in production; `http` only for local development.
    pub relay_scheme: RelayScheme,
    /// Scheme of the URL providers deliver to, reported as `delivery_url`.
    pub public_scheme: RelayScheme,
    pub max_body_bytes: usize,
    /// Accepted skew between a provider's signed timestamp and gateway time.
    pub timestamp_tolerance_seconds: i64,
    pub forward_concurrency: usize,
    /// Queued deliveries older than this fail instead of retrying.
    pub retry_max_age_seconds: i64,
    /// Terminal deliveries and applied operations are reaped after this many days.
    pub retention_days: i64,
    pub rate_limit_per_minute: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayScheme {
    Https,
    Http,
}

impl RelayScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            RelayScheme::Https => "https",
            RelayScheme::Http => "http",
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_map(&std::env::vars().collect())
    }

    pub fn from_map(env: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let listen_addr = env
            .get("BUZZ_WEBHOOK_LISTEN_ADDR")
            .map(String::as_str)
            .unwrap_or("0.0.0.0:8080")
            .parse()
            .map_err(|_| ConfigError::Invalid("BUZZ_WEBHOOK_LISTEN_ADDR"))?;
        let private_addr = env
            .get("BUZZ_WEBHOOK_PRIVATE_ADDR")
            .map(String::as_str)
            .unwrap_or("0.0.0.0:8081")
            .parse()
            .map_err(|_| ConfigError::Invalid("BUZZ_WEBHOOK_PRIVATE_ADDR"))?;
        let signing_keys = Keys::parse(req(env, "BUZZ_WEBHOOK_SIGNING_KEY")?)
            .map_err(|_| ConfigError::Invalid("BUZZ_WEBHOOK_SIGNING_KEY"))?;
        let secret_keyring = parse_keyring(
            req(env, "BUZZ_WEBHOOK_SECRET_KEYRING")?,
            "BUZZ_WEBHOOK_SECRET_KEYRING",
        )?;
        let relay_pubkeys = req(env, "BUZZ_WEBHOOK_RELAY_PUBKEYS")?
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                PublicKey::parse(s).map_err(|_| ConfigError::Invalid("BUZZ_WEBHOOK_RELAY_PUBKEYS"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if relay_pubkeys.is_empty() {
            return Err(ConfigError::Invalid("BUZZ_WEBHOOK_RELAY_PUBKEYS"));
        }
        let relay_scheme = scheme(env, "BUZZ_WEBHOOK_RELAY_SCHEME")?;
        let public_scheme = scheme(env, "BUZZ_WEBHOOK_PUBLIC_SCHEME")?;
        Ok(Self {
            listen_addr,
            private_addr,
            database_url: req(env, "DATABASE_URL")?.to_owned(),
            signing_keys,
            secret_keyring,
            relay_pubkeys,
            relay_scheme,
            public_scheme,
            max_body_bytes: bounded_positive(env, "BUZZ_WEBHOOK_MAX_BODY_BYTES", 1 << 20, 16 << 20)?
                as usize,
            timestamp_tolerance_seconds: bounded_positive(
                env,
                "BUZZ_WEBHOOK_TIMESTAMP_TOLERANCE_SECS",
                300,
                3_600,
            )?,
            forward_concurrency: bounded_positive(
                env,
                "BUZZ_WEBHOOK_FORWARD_CONCURRENCY",
                16,
                1_024,
            )? as usize,
            retry_max_age_seconds: bounded_positive(
                env,
                "BUZZ_WEBHOOK_RETRY_MAX_AGE_SECS",
                86_400,
                7 * 86_400,
            )?,
            retention_days: bounded_positive(env, "BUZZ_WEBHOOK_RETENTION_DAYS", 14, 365)?,
            rate_limit_per_minute: bounded_positive(
                env,
                "BUZZ_WEBHOOK_RATE_LIMIT_PER_MIN",
                600,
                1_000_000,
            )? as u32,
        })
    }
}

fn req<'a>(env: &'a HashMap<String, String>, key: &'static str) -> Result<&'a str, ConfigError> {
    env.get(key)
        .map(String::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or(ConfigError::Missing(key))
}

fn bounded_positive(
    env: &HashMap<String, String>,
    key: &'static str,
    default: i64,
    max: i64,
) -> Result<i64, ConfigError> {
    let Some(raw) = env.get(key) else {
        return Ok(default);
    };
    let value: i64 = raw.trim().parse().map_err(|_| ConfigError::Invalid(key))?;
    if value < 1 || value > max {
        return Err(ConfigError::Invalid(key));
    }
    Ok(value)
}

/// Parse `id:base64-32-bytes,id2:base64-32-bytes`. Ids must be unique.
pub fn parse_keyring(raw: &str, key: &'static str) -> Result<Vec<KeyConfig>, ConfigError> {
    let mut keys = Vec::new();
    for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (id, encoded) = entry.split_once(':').ok_or(ConfigError::Invalid(key))?;
        let id = id.trim();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || keys.iter().any(|k: &KeyConfig| k.id == id)
        {
            return Err(ConfigError::Invalid(key));
        }
        let decoded = STANDARD
            .decode(encoded.trim())
            .map_err(|_| ConfigError::Invalid(key))?;
        if decoded.len() != 32 {
            return Err(ConfigError::Invalid(key));
        }
        keys.push(KeyConfig {
            id: id.to_owned(),
            key: decoded,
        });
    }
    if keys.is_empty() {
        return Err(ConfigError::Invalid(key));
    }
    Ok(keys)
}

fn scheme(env: &HashMap<String, String>, name: &'static str) -> Result<RelayScheme, ConfigError> {
    match env.get(name).map(String::as_str).unwrap_or("https") {
        "https" => Ok(RelayScheme::Https),
        "http" => Ok(RelayScheme::Http),
        _ => Err(ConfigError::Invalid(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNING_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const RELAY_PUBKEY: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    fn base() -> HashMap<String, String> {
        let key = STANDARD.encode([7u8; 32]);
        HashMap::from([
            (
                "DATABASE_URL".to_owned(),
                "postgres://localhost/ingress".to_owned(),
            ),
            (
                "BUZZ_WEBHOOK_SIGNING_KEY".to_owned(),
                SIGNING_KEY.to_owned(),
            ),
            (
                "BUZZ_WEBHOOK_SECRET_KEYRING".to_owned(),
                format!("k1:{key}"),
            ),
            (
                "BUZZ_WEBHOOK_RELAY_PUBKEYS".to_owned(),
                RELAY_PUBKEY.to_owned(),
            ),
        ])
    }

    #[test]
    fn defaults_apply_when_optional_variables_are_absent() {
        let c = Config::from_map(&base()).unwrap();
        assert_eq!(c.listen_addr.port(), 8080);
        assert_eq!(c.private_addr.port(), 8081);
        assert_eq!(c.relay_scheme, RelayScheme::Https);
        assert_eq!(c.public_scheme, RelayScheme::Https);
        assert_eq!(c.max_body_bytes, 1 << 20);
        assert_eq!(c.timestamp_tolerance_seconds, 300);
        assert_eq!(c.forward_concurrency, 16);
        assert_eq!(c.retry_max_age_seconds, 86_400);
        assert_eq!(c.retention_days, 14);
        assert_eq!(c.rate_limit_per_minute, 600);
        assert_eq!(c.secret_keyring.len(), 1);
        assert_eq!(c.relay_pubkeys.len(), 1);
    }

    #[test]
    fn required_variables_are_enforced() {
        for key in [
            "DATABASE_URL",
            "BUZZ_WEBHOOK_SIGNING_KEY",
            "BUZZ_WEBHOOK_SECRET_KEYRING",
            "BUZZ_WEBHOOK_RELAY_PUBKEYS",
        ] {
            let mut env = base();
            env.remove(key);
            assert_eq!(
                Config::from_map(&env).unwrap_err(),
                ConfigError::Missing(key)
            );
        }
    }

    #[test]
    fn invalid_values_are_rejected() {
        let cases = [
            ("BUZZ_WEBHOOK_LISTEN_ADDR", "not-an-addr"),
            ("BUZZ_WEBHOOK_SIGNING_KEY", "zz"),
            ("BUZZ_WEBHOOK_SECRET_KEYRING", "k1:short"),
            ("BUZZ_WEBHOOK_RELAY_PUBKEYS", "nothex"),
            ("BUZZ_WEBHOOK_RELAY_SCHEME", "ftp"),
            ("BUZZ_WEBHOOK_MAX_BODY_BYTES", "0"),
            ("BUZZ_WEBHOOK_RETENTION_DAYS", "9999"),
            ("BUZZ_WEBHOOK_FORWARD_CONCURRENCY", "-1"),
        ];
        for (key, value) in cases {
            let mut env = base();
            env.insert(key.to_owned(), value.to_owned());
            assert_eq!(
                Config::from_map(&env).unwrap_err(),
                ConfigError::Invalid(key),
                "{key}"
            );
        }
    }

    #[test]
    fn keyring_rejects_duplicate_ids_and_accepts_multiple_keys() {
        let a = STANDARD.encode([1u8; 32]);
        let b = STANDARD.encode([2u8; 32]);
        let keys = parse_keyring(&format!("a:{a}, b:{b}"), "K").unwrap();
        assert_eq!(
            keys.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(
            parse_keyring(&format!("a:{a},a:{b}"), "K"),
            Err(ConfigError::Invalid("K"))
        );
    }
}
