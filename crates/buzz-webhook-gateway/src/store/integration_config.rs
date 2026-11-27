//! What an integration does with a delivery: how it finds destination
//! channels, what it posts, and how it groups related deliveries. Every
//! expression is CEL over `event` and `payload`, checked by the management
//! API before it is stored.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How a delivery finds its destination channels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Each subscription's filter decides whether its channel receives the
    /// delivery.
    #[default]
    ChannelSubscriptions,
    /// [`IntegrationConfig::channel_expr`] resolves the payload to one
    /// destination channel.
    CallerSelectsChannel,
}

impl DeliveryMode {
    /// Stable storage and wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChannelSubscriptions => "channel_subscriptions",
            Self::CallerSelectsChannel => "caller_selects_channel",
        }
    }

    /// Inverse of [`DeliveryMode::as_str`].
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "channel_subscriptions" => Some(Self::ChannelSubscriptions),
            "caller_selects_channel" => Some(Self::CallerSelectsChannel),
            _ => None,
        }
    }
}

/// An integration's delivery mode and CEL expressions. Each field defaults so
/// responses stored before it existed still deserialize.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationConfig {
    #[serde(default)]
    pub delivery_mode: DeliveryMode,
    /// Resolves the destination channel id. Required for
    /// [`DeliveryMode::CallerSelectsChannel`].
    #[serde(default)]
    pub channel_expr: Option<String>,
    /// Renders the message text; `None` sends the default message.
    #[serde(default)]
    pub message_expr: Option<String>,
    /// Produces the structured payload of the message's `message-data` tag.
    #[serde(default)]
    pub message_data_expr: Option<String>,
    /// Extracts the key that groups related deliveries, such as an incident.
    #[serde(default)]
    pub group_key_expr: Option<String>,
}

impl IntegrationConfig {
    /// The expressions in a fixed order, named by their request field.
    pub fn expressions(&self) -> [(&'static str, Option<&str>); 4] {
        [
            ("channel_expr", self.channel_expr.as_deref()),
            ("message_expr", self.message_expr.as_deref()),
            ("message_data_expr", self.message_data_expr.as_deref()),
            ("group_key_expr", self.group_key_expr.as_deref()),
        ]
    }

    /// Feed every field into an operation fingerprint. Each expression is
    /// length-prefixed so no expression collides with another request's, and
    /// absent differs from every expression.
    pub(super) fn fingerprint(&self, hasher: &mut Sha256) {
        hasher.update(self.delivery_mode.as_str().as_bytes());
        hasher.update(b"\0");
        for (_, expression) in self.expressions() {
            match expression {
                None => hasher.update(b"\0"),
                Some(source) => {
                    hasher.update(b"\x01");
                    hasher.update(
                        u64::try_from(source.len())
                            .unwrap_or(u64::MAX)
                            .to_be_bytes(),
                    );
                    hasher.update(source.as_bytes());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_changes_the_fingerprint() {
        let digest = |config: &IntegrationConfig| {
            let mut hasher = Sha256::new();
            config.fingerprint(&mut hasher);
            hasher.finalize()
        };
        let base = IntegrationConfig::default();
        let some = || Some("payload.x".to_owned());
        let variants = [
            IntegrationConfig {
                delivery_mode: DeliveryMode::CallerSelectsChannel,
                ..base.clone()
            },
            IntegrationConfig {
                channel_expr: some(),
                ..base.clone()
            },
            IntegrationConfig {
                message_expr: some(),
                ..base.clone()
            },
            IntegrationConfig {
                message_data_expr: some(),
                ..base.clone()
            },
            IntegrationConfig {
                group_key_expr: some(),
                ..base.clone()
            },
        ];
        let mut seen = vec![digest(&base)];
        for variant in &variants {
            let d = digest(variant);
            assert!(!seen.contains(&d), "{variant:?}");
            seen.push(d);
        }
        for mode in [
            DeliveryMode::ChannelSubscriptions,
            DeliveryMode::CallerSelectsChannel,
        ] {
            assert_eq!(DeliveryMode::parse(mode.as_str()), Some(mode));
        }
    }
}
