//! Relay client: subscriptions, publishing, and NIP-44 frame handling.
//!
//! The spawner is an ordinary relay client. It authenticates over NIP-42 with
//! its own key, subscribes for the specs addressed to it and the attestation
//! frames sent to it, and publishes status back. It holds no database
//! connection and no privileged relay access.

use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use buzz_core::kind::{KIND_PERSONA, KIND_SPAWNER_AGENT_SPEC, KIND_SPAWNER_ATTESTATION};
use buzz_sdk::spawner::{
    build_spawner_agent_status, build_spawner_announcement, build_spawner_attestation,
    spec_from_event, spec_slug_from_event, AttestationFrame, SpawnerAgentStatus,
    SpawnerAnnouncement, SPAWNER_TAG,
};
use buzz_ws_client::{connection::NostrWsConnection, message::RelayMessage};
use nostr::{Event, Keys, PublicKey};
use serde_json::json;
use tracing::{debug, warn};

use crate::reconcile::DesiredAgent;

/// Subscription id for agent specs addressed to this spawner.
const SUB_SPECS: &str = "spawner-specs";

/// Subscription id for attestation frames addressed to this spawner.
const SUB_ATTESTATION: &str = "spawner-attestation";

/// How long to wait for a relay frame before yielding to the reconcile timer.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait for a one-shot query to reach EOSE.
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Cap on frames set aside while a one-shot query holds the socket.
const MAX_DEFERRED_FRAMES: usize = 256;

/// Something the daemon needs to act on.
pub enum Inbound {
    /// A spec addressed to this spawner arrived or changed.
    Spec {
        /// Spec author.
        owner_pubkey: String,
        /// The parsed desired agent.
        desired: DesiredAgent,
        /// Event timestamp, so an older revision cannot overwrite a newer one.
        created_at: u64,
    },
    /// A spec was deleted (NIP-09 tombstone or an emptied replacement).
    SpecDeleted {
        /// Spec author.
        owner_pubkey: String,
        /// Spec slug.
        slug: String,
        /// Event timestamp, so a stale tombstone cannot delete a live spec.
        created_at: u64,
    },
    /// An attestation frame arrived, already decrypted.
    Attestation {
        /// Verified event author.
        sender: PublicKey,
        /// The decrypted frame.
        frame: AttestationFrame,
    },
    /// The relay finished replaying stored specs.
    ///
    /// Until this arrives the daemon's desired state is *unknown*, not empty —
    /// a distinction that matters enormously, because acting on an absent spec
    /// means destroying an agent's container, volume, and secret key.
    SpecsHydrated,
    /// Nothing arrived before the receive timeout.
    Idle,
}

/// A relay client scoped to spawner duties.
pub struct SpawnerRelay {
    conn: NostrWsConnection,
    keys: Keys,
    relay_url: String,
    /// Frames received while a one-shot query held the socket, replayed by
    /// [`Self::next`] before anything new is read.
    deferred: VecDeque<RelayMessage>,
}

impl SpawnerRelay {
    /// Connect, authenticate, and install both standing subscriptions.
    pub async fn connect(relay_url: &str, keys: &Keys) -> Result<Self> {
        // No NIP-OA tag: the spawner authenticates as itself, an ordinary relay
        // member. It is not an agent and has no owner.
        let conn = NostrWsConnection::connect_authenticated(relay_url, keys, None)
            .await
            .with_context(|| format!("failed to authenticate to {relay_url}"))?;

        let mut relay = Self {
            conn,
            keys: keys.clone(),
            relay_url: relay_url.to_string(),
            deferred: VecDeque::new(),
        };
        relay.subscribe_all().await?;
        Ok(relay)
    }

    /// Reconnect after a transport failure, restoring both subscriptions.
    pub async fn reconnect(&mut self) -> Result<()> {
        // Anything deferred belonged to the socket that just died. Specs are
        // replayed by the new subscription, and an ephemeral frame held across
        // a disconnect answers a handshake round the reconnect has already
        // outlived, so acting on it would only apply stale state.
        self.deferred.clear();
        self.conn = NostrWsConnection::connect_authenticated(&self.relay_url, &self.keys, None)
            .await
            .with_context(|| format!("failed to reconnect to {}", self.relay_url))?;
        self.subscribe_all().await
    }

    async fn subscribe_all(&mut self) -> Result<()> {
        let me = self.keys.public_key().to_hex();

        // Specs name their target spawner, so this filter admits only work
        // meant for this daemon even when several share a relay.
        self.conn
            .send_raw(&json!([
                "REQ",
                SUB_SPECS,
                {
                    "kinds": [KIND_SPAWNER_AGENT_SPEC],
                    format!("#{SPAWNER_TAG}"): [me],
                }
            ]))
            .await?;

        self.conn
            .send_raw(&json!([
                "REQ",
                SUB_ATTESTATION,
                {
                    "kinds": [KIND_SPAWNER_ATTESTATION],
                    "#p": [me],
                }
            ]))
            .await?;

        Ok(())
    }

    /// Set a frame aside for [`Self::next`], dropping the oldest under flood.
    fn defer(&mut self, msg: RelayMessage) {
        defer_frame(&mut self.deferred, msg);
    }

    /// Wait for the next actionable relay frame.
    pub async fn next(&mut self) -> Result<Inbound> {
        // Frames set aside during a one-shot query come first, so an
        // attestation response that raced a persona fetch is still acted on.
        let msg = match self.deferred.pop_front() {
            Some(msg) => msg,
            None => match self.conn.next_event(RECV_TIMEOUT).await {
                Ok(msg) => msg,
                // A timeout is the common case, not a failure: it just means no
                // event arrived within the window, so the daemon can run its
                // periodic reconcile and come back.
                Err(buzz_ws_client::error::WsClientError::Timeout) => return Ok(Inbound::Idle),
                Err(e) => return Err(e.into()),
            },
        };

        match msg {
            RelayMessage::Event {
                subscription_id,
                event,
            } => self.classify(&subscription_id, *event),
            RelayMessage::Eose { subscription_id } if subscription_id == SUB_SPECS => {
                Ok(Inbound::SpecsHydrated)
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } => bail!("relay closed subscription {subscription_id}: {message}"),
            _ => Ok(Inbound::Idle),
        }
    }

    fn classify(&self, subscription_id: &str, event: Event) -> Result<Inbound> {
        if subscription_id == SUB_ATTESTATION {
            let frame = self.decrypt_frame(&event)?;
            return Ok(Inbound::Attestation {
                sender: event.pubkey,
                frame,
            });
        }

        let owner_pubkey = event.pubkey.to_hex();
        let Some(slug) = spec_slug_from_event(&event) else {
            // A spec without a `d` tag is unaddressable; there is nothing to
            // reconcile it against.
            debug!("dropping spec event {} with no d tag", event.id);
            return Ok(Inbound::Idle);
        };

        // A NIP-33 replacement with empty content is the tombstone convention
        // for parameterized-replaceable events, since a delete leaves nothing
        // to fan out.
        if event.content.trim().is_empty() {
            return Ok(Inbound::SpecDeleted {
                owner_pubkey,
                slug,
                created_at: event.created_at.as_secs(),
            });
        }

        match spec_from_event(&event) {
            Ok(spec) => Ok(Inbound::Spec {
                owner_pubkey: owner_pubkey.clone(),
                desired: DesiredAgent {
                    slug,
                    owner_pubkey,
                    spec,
                },
                created_at: event.created_at.as_secs(),
            }),
            Err(e) => {
                // An invalid spec is the owner's bug, not ours. Log it and keep
                // serving every other agent rather than failing the pass.
                warn!("ignoring invalid spec {owner_pubkey}/{slug}: {e}");
                Ok(Inbound::Idle)
            }
        }
    }

    fn decrypt_frame(&self, event: &Event) -> Result<AttestationFrame> {
        let plaintext = nostr::nips::nip44::decrypt(
            self.keys.secret_key(),
            &event.pubkey,
            event.content.as_str(),
        )
        .context("failed to decrypt attestation frame")?;

        let frame: AttestationFrame =
            serde_json::from_str(&plaintext).context("failed to parse attestation frame")?;
        frame.validate().context("malformed attestation frame")?;
        Ok(frame)
    }

    /// Send an attestation frame, NIP-44 encrypted to `recipient`.
    pub async fn send_attestation(
        &mut self,
        recipient: &PublicKey,
        frame: &AttestationFrame,
    ) -> Result<()> {
        let plaintext =
            serde_json::to_string(frame).context("failed to serialize attestation frame")?;
        let ciphertext = nostr::nips::nip44::encrypt(
            self.keys.secret_key(),
            recipient,
            plaintext,
            nostr::nips::nip44::Version::V2,
        )
        .context("failed to encrypt attestation frame")?;

        let event = build_spawner_attestation(&recipient.to_hex(), &ciphertext)?
            .sign_with_keys(&self.keys)?;
        let ok = self.conn.send_event(event).await?;
        if !ok.accepted {
            bail!("relay rejected attestation frame: {}", ok.message);
        }
        Ok(())
    }

    /// Publish this spawner's announcement so owners can discover it.
    ///
    /// Replaceable and keyed by `(pubkey, kind)`, so republishing on every
    /// reconcile keeps the advertised capacity roughly current without
    /// accumulating events.
    pub async fn publish_announcement(&mut self, announcement: &SpawnerAnnouncement) -> Result<()> {
        let event = build_spawner_announcement(announcement)?.sign_with_keys(&self.keys)?;
        let ok = self.conn.send_event(event).await?;
        if !ok.accepted {
            bail!("relay rejected announcement: {}", ok.message);
        }
        Ok(())
    }

    /// Publish a status event for one agent.
    pub async fn publish_status(
        &mut self,
        slug: &str,
        owner_pubkey: &str,
        status: &SpawnerAgentStatus,
    ) -> Result<()> {
        let event =
            build_spawner_agent_status(slug, owner_pubkey, status)?.sign_with_keys(&self.keys)?;
        let ok = self.conn.send_event(event).await?;
        if !ok.accepted {
            bail!("relay rejected status event: {}", ok.message);
        }
        Ok(())
    }

    /// Tombstone an agent's status so clients stop showing it.
    ///
    /// Kind 30179 is replaceable, so a deleted agent's last status — often
    /// `pending_attestation` — would otherwise persist forever and every client
    /// would keep rendering a row for an agent that no longer exists. An
    /// emptied replacement is the tombstone convention for a
    /// parameterized-replaceable kind; a kind:5 deletion leaves nothing to fan
    /// out, so subscribers would never learn.
    ///
    /// Built directly rather than through the SDK builder because that
    /// validates a status body, which a tombstone deliberately has none of.
    pub async fn tombstone_status(&mut self, slug: &str, owner_pubkey: &str) -> Result<()> {
        let event = nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core::kind::KIND_SPAWNER_AGENT_STATUS as u16),
            "",
        )
        .tags(vec![
            nostr::Tag::parse(["d", slug])?,
            nostr::Tag::parse(["p", owner_pubkey])?,
        ])
        .sign_with_keys(&self.keys)?;
        let ok = self.conn.send_event(event).await?;
        if !ok.accepted {
            bail!("relay rejected status tombstone: {}", ok.message);
        }
        Ok(())
    }

    /// Fetch the personas authored by `owner`, keyed by `d` tag.
    ///
    /// Only reaches personas published `["shared","true"]`. Kind 30175 is
    /// author-only otherwise, and the spawner authenticates as itself with no
    /// NIP-OA tag (see [`Self::connect`]) — the owner delegation the relay
    /// applies to attested readers covers the *agents* a spawner runs, never
    /// the spawner. An unshared persona therefore has to arrive over the
    /// encrypted attestation handshake instead; [`crate::daemon`] falls back to
    /// that and says so when neither source has one.
    pub async fn fetch_personas(&mut self, owner: &PublicKey) -> Result<HashMap<String, Event>> {
        let sub_id = format!("personas-{}", &owner.to_hex()[..8]);
        self.conn
            .send_raw(&json!([
                "REQ",
                sub_id,
                { "kinds": [KIND_PERSONA], "authors": [owner.to_hex()] }
            ]))
            .await?;

        let mut personas = HashMap::new();
        let deadline = tokio::time::Instant::now() + QUERY_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                warn!("persona query for {} timed out", owner.to_hex());
                break;
            }
            match self.conn.next_event(remaining).await {
                Ok(RelayMessage::Event {
                    subscription_id,
                    event,
                }) if subscription_id == sub_id => {
                    if let Some(d) = spec_slug_from_event(&event) {
                        personas.insert(d, *event);
                    }
                }
                Ok(RelayMessage::Eose { subscription_id }) if subscription_id == sub_id => break,
                Ok(RelayMessage::Closed {
                    subscription_id,
                    message,
                }) if subscription_id == sub_id => {
                    bail!("relay closed persona query: {message}");
                }
                // Frames for the standing subscriptions interleave with this
                // one-shot query. They cannot be dropped: kind:24201 is
                // ephemeral, so an attestation response that arrives while a
                // persona query is in flight is gone for good, and the agent it
                // answers for sits in pending_attestation until the handshake
                // times out and the owner is asked to approve all over again.
                // Defer them to the next `next()` call instead.
                Ok(other) => {
                    self.defer(other);
                    continue;
                }
                Err(buzz_ws_client::error::WsClientError::Timeout) => break,
                Err(e) => return Err(e.into()),
            }
        }

        let _ = self.conn.send_raw(&json!(["CLOSE", sub_id])).await;
        Ok(personas)
    }
}

/// Push `msg` onto the deferral queue, evicting the oldest when full.
///
/// Bounded because anyone on the relay can address frames at this spawner; an
/// unbounded queue would be a memory-growth lever for them. Eviction takes the
/// oldest, so a flood cannot push out a frame that just arrived.
fn defer_frame(deferred: &mut VecDeque<RelayMessage>, msg: RelayMessage) {
    if deferred.len() >= MAX_DEFERRED_FRAMES {
        deferred.pop_front();
        warn!("deferred-frame queue is full; dropping the oldest frame");
    }
    deferred.push_back(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eose(id: &str) -> RelayMessage {
        RelayMessage::Eose {
            subscription_id: id.to_string(),
        }
    }

    fn id_of(msg: &RelayMessage) -> String {
        match msg {
            RelayMessage::Eose { subscription_id } => subscription_id.clone(),
            _ => unreachable!("test only defers Eose frames"),
        }
    }

    #[test]
    fn deferred_frames_replay_in_arrival_order() {
        // An attestation response that raced a persona query must still be
        // acted on, and in the order the relay sent it.
        let mut deferred = VecDeque::new();
        defer_frame(&mut deferred, eose("first"));
        defer_frame(&mut deferred, eose("second"));

        assert_eq!(id_of(&deferred.pop_front().unwrap()), "first");
        assert_eq!(id_of(&deferred.pop_front().unwrap()), "second");
        assert!(deferred.is_empty());
    }

    #[test]
    fn deferral_is_bounded_and_evicts_the_oldest() {
        // Anyone can address frames at this spawner, so the queue must not be
        // an unbounded allocation lever — and the newest frame, which is the
        // one most likely to still matter, must survive the flood.
        let mut deferred = VecDeque::new();
        for i in 0..MAX_DEFERRED_FRAMES + 10 {
            defer_frame(&mut deferred, eose(&i.to_string()));
        }

        assert_eq!(deferred.len(), MAX_DEFERRED_FRAMES);
        assert_eq!(id_of(deferred.front().unwrap()), "10");
        assert_eq!(
            id_of(deferred.back().unwrap()),
            (MAX_DEFERRED_FRAMES + 9).to_string()
        );
    }
}
