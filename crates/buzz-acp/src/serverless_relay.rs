//! Serverless relay backend built on `nostr-relay-pool`.
//!
//! In serverless mode the agent is an ordinary Nostr client talking to generic
//! public relays (no Buzz server, no NIP-42 AUTH, no HTTP bridge). Rather
//! than hand-roll multi-relay subscribe / reconnect / resubscribe / dedup (the
//! `relay.rs` background-task machinery used for server mode), we delegate to
//! [`RelayPool`], which provides exactly the standard Nostr client behaviour:
//!
//! - **Persistent REQ per relay**, fanned out across every relay in the pool.
//! - **Auto-reconnect** (on by default) with jittered backoff.
//! - **Auto-resubscribe** on reconnect — the pool re-sends every active REQ, and
//!   relays replay their stored events matching the filter (NIP-01), so events
//!   that arrived during a disconnect are recovered.
//! - **Merge + dedup** across relays: `RelayPoolNotification::Event` fires only
//!   the first time an event id is seen, and excludes events we published.
//!
//! This mirrors how damus (`RelayPool`), 0xchat, and nostr-tools (`SimplePool`)
//! receive messages reliably across flaky relays.
//!
//! Subscription IDs match the server-mode scheme so the harness routes events
//! identically: `ch-<uuid>` for channels, plus the fixed membership / observer
//! / gift-wrap inbox ids.

use nostr::{Event, Filter, Keys, Kind, PublicKey, SingleLetterTag, SubscriptionId};
use nostr_relay_pool::{RelayOptions, RelayPool, RelayPoolNotification, SubscribeOptions};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::ChannelFilter;
use crate::relay::{
    channel_sub_id, extract_h_tag_uuid, rumor_to_event, BuzzEvent, RelayError, GIFT_WRAP_SUB_ID,
    MEMBERSHIP_NOTIF_SUB_ID, OBSERVER_CONTROL_SUB_ID,
};

/// Serverless relay backend. Owns a [`RelayPool`] and a background task that
/// converts pool notifications into [`BuzzEvent`]s.
///
/// One-shot queries (channel discovery) go through `RestClient`, which shares
/// this pool via [`ServerlessRelay::pool`] — so there's no separate fetch path
/// here.
pub struct ServerlessRelay {
    pool: RelayPool,
    agent_pubkey: PublicKey,
    /// Merged, classified channel/membership/gift-wrap events.
    event_rx: mpsc::Receiver<Option<BuzzEvent>>,
    /// Relay URLs the pool is configured with (for diagnostics).
    relay_urls: Vec<String>,
    /// Background notification-pump handle.
    _pump: tokio::task::JoinHandle<()>,
}

impl ServerlessRelay {
    /// Build the pool, add all relays, connect, and spawn the notification pump.
    pub async fn connect(
        relay_url: &str,
        keys: &Keys,
        agent_pubkey_hex: &str,
    ) -> Result<Self, RelayError> {
        let relay_urls: Vec<String> = relay_url
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if relay_urls.is_empty() {
            return Err(RelayError::Http("no relay URLs provided".into()));
        }

        let agent_pubkey = PublicKey::from_hex(agent_pubkey_hex)
            .map_err(|e| RelayError::Http(format!("invalid agent pubkey: {e}")))?;

        let pool = RelayPool::new();
        let mut added = 0usize;
        for url in &relay_urls {
            // RelayOptions::default() has reconnect=true + adjusted (jittered)
            // retry interval — exactly the auto-reconnect we want.
            match pool.add_relay(url.as_str(), RelayOptions::default()).await {
                Ok(_) => added += 1,
                Err(e) => warn!("serverless: failed to add relay {url}: {e}"),
            }
        }
        if added == 0 {
            return Err(RelayError::Http("no relays could be added".into()));
        }

        // Connects all relays in the background; reconnection is automatic.
        pool.connect().await;
        info!(
            "serverless: relay pool connecting to {}/{} relay(s): {}",
            added,
            relay_urls.len(),
            relay_urls.join(", ")
        );

        let (event_tx, event_rx) = mpsc::channel::<Option<BuzzEvent>>(1024);

        let pump_keys = keys.clone();
        let notifications = pool.notifications();
        let pump = tokio::spawn(notification_pump(notifications, pump_keys, event_tx));

        Ok(Self {
            pool,
            agent_pubkey,
            event_rx,
            relay_urls,
            _pump: pump,
        })
    }

    /// Relay URLs the pool is using.
    pub fn relay_urls(&self) -> &[String] {
        &self.relay_urls
    }

    /// Cheap clone of the underlying pool (Arc inside) so detached tasks
    /// (presence, typing) can publish ephemeral events without holding the
    /// whole `ServerlessRelay`.
    pub fn pool(&self) -> RelayPool {
        self.pool.clone()
    }

    /// Next merged event (channel / membership / decrypted gift-wrap). Returns
    /// `None` only if the pump task ended (pool shut down).
    pub async fn next_event(&mut self) -> Option<BuzzEvent> {
        // The pump sends `Some(event)`; it never sends `None` unless closing.
        match self.event_rx.recv().await {
            Some(Some(ev)) => Some(ev),
            Some(None) | None => None,
        }
    }

    /// Publish a signed event to all relays (succeeds if any accepts).
    pub async fn publish(&self, event: &Event) -> Result<(), RelayError> {
        let output = self
            .pool
            .send_event(event)
            .await
            .map_err(|e| RelayError::Http(format!("publish failed: {e}")))?;
        if output.success.is_empty() {
            return Err(RelayError::Http(format!(
                "no relay accepted event (failed on {} relay(s))",
                output.failed.len()
            )));
        }
        Ok(())
    }

    /// Subscribe to a channel with the standard `ch-<uuid>` id. The pool keeps
    /// the REQ open and resubscribes on reconnect automatically.
    pub async fn subscribe_channel(
        &self,
        channel_id: Uuid,
        filter: ChannelFilter,
    ) -> Result<(), RelayError> {
        let id = SubscriptionId::new(channel_sub_id(channel_id));
        let mut f = Filter::new().custom_tag(
            SingleLetterTag::lowercase(nostr::Alphabet::H),
            channel_id.to_string(),
        );
        if let Some(kinds) = filter.kinds {
            f = f.kinds(kinds.into_iter().map(|k| Kind::from(k as u16)));
        }
        if filter.require_mention {
            f = f.pubkey(self.agent_pubkey);
        }
        self.pool
            .subscribe_with_id(id, f, SubscribeOptions::default())
            .await
            .map_err(|e| RelayError::Http(format!("subscribe failed: {e}")))?;
        debug!("serverless: subscribed to channel {channel_id}");
        Ok(())
    }

    /// Unsubscribe a channel.
    pub async fn unsubscribe_channel(&self, channel_id: Uuid) {
        let id = SubscriptionId::new(channel_sub_id(channel_id));
        self.pool.unsubscribe(&id).await;
    }

    /// Subscribe to the agent's NIP-17 gift-wrap inbox (kind 1059, #p = agent).
    pub async fn subscribe_gift_wrap(&self) -> Result<(), RelayError> {
        let id = SubscriptionId::new(GIFT_WRAP_SUB_ID);
        let f = Filter::new().kind(Kind::GiftWrap).pubkey(self.agent_pubkey);
        self.pool
            .subscribe_with_id(id, f, SubscribeOptions::default())
            .await
            .map_err(|e| RelayError::Http(format!("gift-wrap subscribe failed: {e}")))?;
        Ok(())
    }

    /// Disconnect / shut down the pool.
    pub async fn shutdown(&self) {
        self.pool.shutdown().await;
    }
}

/// Classify pool notifications into BuzzEvents / observer frames, mirroring
/// the server-mode bg-task routing — but with no manual dedup/reconnect (the
/// pool handles both).
async fn notification_pump(
    mut notifications: tokio::sync::broadcast::Receiver<RelayPoolNotification>,
    keys: Keys,
    event_tx: mpsc::Sender<Option<BuzzEvent>>,
) {
    loop {
        let notif = match notifications.recv().await {
            Ok(n) => n,
            // Lagged: we missed some — keep going (relays will replay on any
            // resubscribe; dedup is by id so no harm).
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!("serverless: notification stream lagged by {n}");
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                let _ = event_tx.send(None).await;
                return;
            }
        };

        match notif {
            RelayPoolNotification::Event {
                subscription_id,
                event,
                ..
            } => {
                let sub = subscription_id.as_str();
                if sub == GIFT_WRAP_SUB_ID {
                    // NIP-17 encrypted message: unwrap to the inner rumor.
                    match nostr::nips::nip59::UnwrappedGift::from_gift_wrap(&keys, &event).await {
                        Ok(unwrapped) => {
                            if let Some(inner) = rumor_to_event(&unwrapped.rumor, unwrapped.sender)
                            {
                                if let Some(channel_uuid) = extract_h_tag_uuid(&inner) {
                                    let _ = event_tx
                                        .send(Some(BuzzEvent {
                                            channel_id: channel_uuid,
                                            event: inner,
                                        }))
                                        .await;
                                }
                            }
                        }
                        Err(e) => debug!("serverless: gift wrap not for us: {e}"),
                    }
                } else if sub == OBSERVER_CONTROL_SUB_ID {
                    // Observer controls are a Sprout-relay feature; never
                    // subscribed in serverless, so this branch is unreachable.
                } else if sub == MEMBERSHIP_NOTIF_SUB_ID {
                    if let Some(channel_uuid) = extract_h_tag_uuid(&event) {
                        let _ = event_tx
                            .send(Some(BuzzEvent {
                                channel_id: channel_uuid,
                                event: *event,
                            }))
                            .await;
                    }
                } else if let Some(channel_uuid) = channel_uuid_from_sub(sub) {
                    let _ = event_tx
                        .send(Some(BuzzEvent {
                            channel_id: channel_uuid,
                            event: *event,
                        }))
                        .await;
                }
            }
            RelayPoolNotification::Message { .. } => {}
            RelayPoolNotification::Shutdown => {
                let _ = event_tx.send(None).await;
                return;
            }
        }
    }
}

/// Parse `ch-<uuid>` subscription ids.
fn channel_uuid_from_sub(sub: &str) -> Option<Uuid> {
    sub.strip_prefix("ch-")
        .and_then(|s| Uuid::parse_str(s).ok())
}
