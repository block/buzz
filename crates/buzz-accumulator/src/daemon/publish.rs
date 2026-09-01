//! Publishing artifacts back into channels as ordinary messages.
//!
//! This is the composition seam: a published artifact is just a kind-9
//! message in a channel the mirror watches, so another fold can select it —
//! folds all the way down, no rollup machinery needed.

use futures_util::future::BoxFuture;
use nostr::{Keys, Tag};

/// One publish, bounded end to end (connect + auth + EVENT + OK).
const PUBLISH_TIMEOUT_SECS: u64 = 30;

/// Publishes one message to a channel as the daemon identity.
pub trait Publisher: Send + Sync {
    /// Publish `content` as a kind-9 message in `channel` (a channel UUID);
    /// resolves to the accepted event id.
    fn publish<'a>(
        &'a self,
        channel: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<String, String>>;
}

/// Real publisher: a short-lived authenticated connection per publish. Same
/// key as the mirror's read connection, publish-only, no REQs — the pattern
/// the relay expects (events bind to the connection's NIP-42 identity).
pub struct RelayPublisher {
    /// Relay websocket URL (same pinned relay the mirror reads from).
    pub relay_url: String,
    /// Signing identity (the person's key).
    pub keys: Keys,
    /// Optional NIP-OA ownership tag (agent identities only).
    pub auth_tag: Option<Tag>,
}

impl Publisher for RelayPublisher {
    fn publish<'a>(
        &'a self,
        channel: &'a str,
        content: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let channel = uuid::Uuid::parse_str(channel)
                .map_err(|e| format!("channel is not a UUID: {e}"))?;
            let event = buzz_sdk::build_message(channel, content, None, &[], false, &[])
                .map_err(|e| format!("building message: {e}"))?
                .sign_with_keys(&self.keys)
                .map_err(|e| format!("signing message: {e}"))?;
            let ok = buzz_ws_client::publish_event(
                &self.relay_url,
                event,
                &self.keys,
                self.auth_tag.as_ref(),
                PUBLISH_TIMEOUT_SECS,
            )
            .await
            .map_err(|e| format!("publishing to relay: {e}"))?;
            if !ok.accepted {
                return Err(format!("relay refused the message: {}", ok.message));
            }
            Ok(ok.event_id)
        })
    }
}
