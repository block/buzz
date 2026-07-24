//! Transport seam wiring — the Nostr relay as one [`Transport`] among many.
//!
//! [`HarnessRelay`] (the NIP-42 relay WebSocket client) implements
//! [`buzz_transport::Transport`] here, making the relay connection just one
//! implementation of the seam. [`connect_transport`] selects an
//! implementation from the environment. The harness's main event loop does
//! not call it yet — it still constructs [`HarnessRelay`] directly, and
//! converting it to run on `Box<dyn Transport>` is the tracked follow-up
//! that makes these variables take effect:
//!
//! | Variable | Values | Meaning |
//! |---|---|---|
//! | `BUZZ_TRANSPORT` | `nostr` (default) \| `remote` | Which transport carries events |
//! | `BUZZ_TRANSPORT_URL` | `wss://…` \| `unix:///…` | Remote bridge endpoint (required for `remote`) |
//! | `BUZZ_TRANSPORT_TOKEN` | any string | Optional bearer token for the bridge |
//! | `BUZZ_TRANSPORT_SOCKS_PROXY` | `socks5://[user:pass@]host:port` | Optional SOCKS5 tunnel to the bridge |
//! | `BUZZ_TRANSPORT_ALLOW_INSECURE` | `true`/`1` | Permit `ws://` to non-loopback hosts |
//!
//! The remote protocol is documented in `crates/buzz-transport/PROTOCOL.md`.

use buzz_transport::remote::{RemoteTransport, RemoteTransportConfig};
use buzz_transport::{
    BoxFuture, SignedEvent, Subscription, Transport, TransportError, TransportEvent,
};
use nostr::Keys;
use tracing::warn;
use uuid::Uuid;

use crate::config::ChannelFilter;
use crate::relay::{HarnessRelay, RelayError};

/// Which [`Transport`] implementation carries events for this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportKind {
    /// Buzz relay over a NIP-42-authenticated WebSocket (the default).
    #[default]
    Nostr,
    /// Operator-supplied bridge speaking the buzz-transport wire protocol.
    Remote,
}

impl std::str::FromStr for TransportKind {
    type Err = TransportSetupError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "nostr" => Ok(Self::Nostr),
            "remote" => Ok(Self::Remote),
            other => Err(TransportSetupError::UnknownKind(other.to_string())),
        }
    }
}

impl TransportKind {
    /// Read `BUZZ_TRANSPORT` from the environment (absent = `Nostr`).
    pub fn from_env() -> Result<Self, TransportSetupError> {
        match std::env::var("BUZZ_TRANSPORT") {
            Ok(value) => value.parse(),
            Err(_) => Ok(Self::default()),
        }
    }
}

/// Errors selecting or establishing a transport.
#[derive(Debug, thiserror::Error)]
pub enum TransportSetupError {
    /// `BUZZ_TRANSPORT` is set to a value this build does not know.
    #[error("unknown BUZZ_TRANSPORT value {0:?} (expected \"nostr\" or \"remote\")")]
    UnknownKind(String),
    /// `BUZZ_TRANSPORT=remote` without `BUZZ_TRANSPORT_URL`.
    #[error("BUZZ_TRANSPORT=remote requires BUZZ_TRANSPORT_URL")]
    MissingUrl,
    /// The selected transport failed to connect.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The Nostr relay connection failed.
    #[error("relay connection failed: {0}")]
    Relay(#[from] RelayError),
}

/// Connect the transport selected by the environment (see module docs).
///
/// `relay_url`, `keys`, and `auth_tag` are the same values
/// [`HarnessRelay::connect`] takes; the remote transport reuses `keys` for
/// its `hello` identity and reads its endpoint configuration from
/// `BUZZ_TRANSPORT_URL` / `BUZZ_TRANSPORT_TOKEN` /
/// `BUZZ_TRANSPORT_ALLOW_INSECURE`.
pub async fn connect_transport(
    kind: TransportKind,
    relay_url: &str,
    keys: &Keys,
    auth_tag: Option<nostr::Tag>,
) -> Result<Box<dyn Transport>, TransportSetupError> {
    match kind {
        TransportKind::Nostr => {
            let pubkey_hex = keys.public_key().to_hex();
            let relay = HarnessRelay::connect(relay_url, keys, &pubkey_hex, auth_tag).await?;
            Ok(Box::new(relay))
        }
        TransportKind::Remote => {
            let url =
                std::env::var("BUZZ_TRANSPORT_URL").map_err(|_| TransportSetupError::MissingUrl)?;
            let config = RemoteTransportConfig {
                url,
                pubkey: keys.public_key().to_hex(),
                token: std::env::var("BUZZ_TRANSPORT_TOKEN").ok(),
                allow_insecure: env_flag("BUZZ_TRANSPORT_ALLOW_INSECURE"),
                socks_proxy: std::env::var("BUZZ_TRANSPORT_SOCKS_PROXY").ok(),
            };
            Ok(Box::new(RemoteTransport::connect(config).await?))
        }
    }
}

/// True when the env var is set to a truthy value (`true`/`1`/`yes`/`on`).
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"))
        .unwrap_or(false)
}

/// Map a [`Subscription`] onto the relay client's [`ChannelFilter`].
fn to_channel_filter(subscription: &Subscription) -> ChannelFilter {
    ChannelFilter {
        kinds: subscription.kinds.clone(),
        require_mention: subscription.require_mention,
    }
}

fn relay_error(e: RelayError) -> TransportError {
    match e {
        RelayError::ConnectionClosed => TransportError::Closed,
        RelayError::Json(e) => TransportError::Codec(e.to_string()),
        other => TransportError::Connection(other.to_string()),
    }
}

/// The Buzz relay connection is one [`Transport`]: subscriptions become
/// NIP-01 `REQ` filters, published events go out on the relay socket, and
/// inbound channel events arrive through the same background task that has
/// always fed the harness.
impl Transport for HarnessRelay {
    fn subscribe(
        &mut self,
        subscription: Subscription,
    ) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            let filter = to_channel_filter(&subscription);
            self.subscribe_channel_from(subscription.channel_id, filter, subscription.replay_since)
                .await
                .map_err(relay_error)
        })
    }

    fn unsubscribe(&mut self, channel_id: Uuid) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            self.unsubscribe_channel(channel_id)
                .await
                .map_err(relay_error)
        })
    }

    fn next_event(&mut self) -> BoxFuture<'_, Option<TransportEvent>> {
        Box::pin(async move {
            // Skip (never terminate on) events that fail conversion — the
            // relay only delivers verified NIP-01 events, so a conversion
            // failure is a bug worth logging, not a stream-ending condition.
            loop {
                let buzz_event = HarnessRelay::next_event(self).await?;
                match SignedEvent::from_nostr(&buzz_event.event) {
                    Ok(event) => {
                        return Some(TransportEvent {
                            channel_id: buzz_event.channel_id,
                            event,
                        });
                    }
                    Err(e) => {
                        warn!(event_id = %buzz_event.event.id, "skipping unconvertible event: {e}");
                    }
                }
            }
        })
    }

    fn publish(&self, event: SignedEvent) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            let event = event.to_nostr()?;
            self.publish_event(event).await.map_err(relay_error)
        })
    }

    fn try_publish(&self, event: SignedEvent) -> Result<(), TransportError> {
        let event = event.to_nostr()?;
        self.try_publish_event(event).map_err(relay_error)
    }

    fn reconnect(&mut self) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move { HarnessRelay::reconnect(self).await.map_err(relay_error) })
    }

    fn shutdown(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move { HarnessRelay::shutdown(*self).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_kind_parses_known_values() {
        assert_eq!(
            "nostr".parse::<TransportKind>().unwrap(),
            TransportKind::Nostr
        );
        assert_eq!(
            "NOSTR".parse::<TransportKind>().unwrap(),
            TransportKind::Nostr
        );
        assert_eq!("".parse::<TransportKind>().unwrap(), TransportKind::Nostr);
        assert_eq!(
            "remote".parse::<TransportKind>().unwrap(),
            TransportKind::Remote
        );
        assert_eq!(
            " Remote ".parse::<TransportKind>().unwrap(),
            TransportKind::Remote
        );
        assert!(matches!(
            "slack".parse::<TransportKind>(),
            Err(TransportSetupError::UnknownKind(_))
        ));
    }

    #[test]
    fn default_transport_is_nostr() {
        assert_eq!(TransportKind::default(), TransportKind::Nostr);
    }

    #[test]
    fn subscription_maps_onto_channel_filter() {
        let sub = Subscription {
            channel_id: Uuid::new_v4(),
            kinds: Some(vec![9, 40002]),
            require_mention: true,
            replay_since: Some(1_700_000_000),
        };
        let filter = to_channel_filter(&sub);
        assert_eq!(filter.kinds, Some(vec![9, 40002]));
        assert!(filter.require_mention);

        let wildcard = to_channel_filter(&Subscription::all(Uuid::new_v4()));
        assert_eq!(wildcard.kinds, None);
        assert!(!wildcard.require_mention);
    }

    #[test]
    fn relay_errors_map_to_transport_errors() {
        assert!(matches!(
            relay_error(RelayError::ConnectionClosed),
            TransportError::Closed
        ));
        assert!(matches!(
            relay_error(RelayError::Timeout),
            TransportError::Connection(_)
        ));
    }
}
