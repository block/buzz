//! NIP-WH delivery intake settings.

use crate::config::ConfigError;

/// Default oldest accepted delivery `created_at`, in seconds (72 hours).
///
/// A gateway buffers deliveries while the relay is unreachable and resends the
/// same signed event, so the general ±900 s window would refuse a delivery
/// queued through any outage longer than 15 minutes.
pub const DEFAULT_DELIVERY_MAX_AGE_SECS: u64 = 72 * 60 * 60;

/// Who may submit NIP-WH deliveries, and how old a delivery may be.
#[derive(Clone, Debug)]
pub struct WebhookIngressConfig {
    /// Keys allowed to sign kind-20701 deliveries
    /// (`BUZZ_WEBHOOK_INGRESS_PUBKEYS`, comma-separated hex). Empty, the
    /// default, turns intake off.
    pub ingress_pubkeys: Vec<nostr::PublicKey>,
    /// Oldest accepted delivery `created_at`, in seconds before now
    /// (`BUZZ_WEBHOOK_DELIVERY_MAX_AGE_SECS`).
    pub delivery_max_age_secs: u64,
}

impl WebhookIngressConfig {
    /// Read the settings from the environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        let var = |name: &str| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(format!("{name} must be valid UTF-8: {error}")),
        };
        (|| {
            Self::parse(
                var("BUZZ_WEBHOOK_INGRESS_PUBKEYS")?.as_deref(),
                var("BUZZ_WEBHOOK_DELIVERY_MAX_AGE_SECS")?.as_deref(),
            )
        })()
        .map_err(ConfigError::InvalidValue)
    }

    fn parse(pubkeys: Option<&str>, max_age: Option<&str>) -> Result<Self, String> {
        let mut ingress_pubkeys = Vec::new();
        for entry in pubkeys.unwrap_or_default().split(',').map(str::trim) {
            if entry.is_empty() {
                continue;
            }
            let valid = entry.len() == 64 && entry.bytes().all(|b| b.is_ascii_hexdigit());
            let key = valid
                .then(|| nostr::PublicKey::from_hex(entry).ok())
                .flatten()
                .ok_or_else(|| {
                    format!(
                        "BUZZ_WEBHOOK_INGRESS_PUBKEYS entry is not a 64-char hex pubkey: {entry:?}"
                    )
                })?;
            if !ingress_pubkeys.contains(&key) {
                ingress_pubkeys.push(key);
            }
        }
        let delivery_max_age_secs = match max_age.map(str::trim).filter(|v| !v.is_empty()) {
            None => DEFAULT_DELIVERY_MAX_AGE_SECS,
            Some(raw) => raw
                .parse::<u64>()
                .ok()
                .filter(|secs| *secs > 0)
                .ok_or_else(|| {
                    format!("BUZZ_WEBHOOK_DELIVERY_MAX_AGE_SECS must be a positive number of seconds: {raw:?}")
                })?,
        };
        Ok(Self {
            ingress_pubkeys,
            delivery_max_age_secs,
        })
    }

    /// Whether `pubkey` may sign deliveries.
    pub fn is_ingress(&self, pubkey: &nostr::PublicKey) -> bool {
        self.ingress_pubkeys.contains(pubkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "7e7e9c42a91bfef19fa929e5fda1b72e0ebc1a4c1141673e2794234d86addf4e";

    #[test]
    fn parses_ingress_settings() {
        let off = WebhookIngressConfig::parse(None, None).unwrap();
        assert!(off.ingress_pubkeys.is_empty());
        assert_eq!(off.delivery_max_age_secs, DEFAULT_DELIVERY_MAX_AGE_SECS);

        let upper = KEY.to_uppercase();
        let on =
            WebhookIngressConfig::parse(Some(&format!(" {KEY}, ,{upper}")), Some("3600")).unwrap();
        let key = nostr::PublicKey::from_hex(KEY).unwrap();
        assert_eq!(on.ingress_pubkeys, vec![key]);
        assert!(on.is_ingress(&key));
        assert_eq!(on.delivery_max_age_secs, 3600);

        for (pubkeys, max_age) in [
            (Some("npub1notvalidhex"), None),
            (Some(&KEY[..63]), None),
            (None, Some("0")),
            (None, Some("72h")),
        ] {
            assert!(
                WebhookIngressConfig::parse(pubkeys, max_age).is_err(),
                "{pubkeys:?} {max_age:?}"
            );
        }
    }
}
