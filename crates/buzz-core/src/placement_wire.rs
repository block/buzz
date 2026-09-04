//! Owner-private immutable placement intent. No transport or execution is enabled.
//!
//! Like NIP-PMA, encryption is owner-to-self: trusted Desktop instances already
//! holding that owner's keys can observe intent for OTHER hosts. Host keys alone
//! cannot decrypt or log in. Never export owner keys to an executor/runtime.
//! The caller supplies current authoritative agent/host bindings; this codec
//! verifies signed scope, not ownership records, revocation or admission.

use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    kind::KIND_PLACEMENT_INTENT,
    placement::{EventOrder, PlacementAction, PlacementIntent},
    relay::normalize_relay_url,
};

/// Separate from the obsolete exact-run `buzz.host.execution.v1` protocol.
pub const NAMESPACE: &str = "buzz.placement.v1";
const MAX_PLAINTEXT: usize = 2048;
const MAX_CIPHERTEXT: usize = 4096;

/// Placement contributions only. Restart is a separate current-host one-shot;
/// Move may issue Start only after ordinary Stop success and validity checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Select the destination; not evidence of a running process.
    Start,
    /// Stop this host without cancelling desired placement on another host.
    Stop,
}

/// All semantic fields are encrypted and bound to ONE signed event identity.
/// No shell, configuration, credentials, run nonce or future Start template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    /// Wire version, currently exactly 1.
    pub v: u8,
    /// Canonical community relay identity, captured from the active session.
    pub community: String,
    /// Authenticated owner, identical to the event author.
    pub owner: PublicKey,
    /// Agent identity, not a local display name.
    pub agent: PublicKey,
    /// Target executor, not necessarily the observing Desktop's executor.
    pub host: PublicKey,
    /// Non-nil request identity; durable deduplication is a separate boundary.
    pub request: Uuid,
    /// Agent + host intent, never an implicitly broadened legacy command.
    pub action: Action,
}

/// Fail-closed codec errors; never include decrypted content or keys.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// Wrong kind/tags, size, event hash or signature.
    #[error("invalid placement envelope")]
    Envelope,
    /// Malformed/unsupported plaintext, including legacy exact-run payloads.
    #[error("invalid placement payload")]
    Payload,
    /// Owner, community, agent or authorized target differs from captured scope.
    #[error("placement scope mismatch")]
    Scope,
    /// Encryption, decryption or signing failed.
    #[error("placement cryptography failed")]
    Crypto,
}

impl Payload {
    fn validate(&self) -> Result<(), Error> {
        if self.v != 1
            || self.request.is_nil()
            || self.community.len() > 512
            || normalize_relay_url(&self.community).ok().as_deref() != Some(&self.community)
        {
            return Err(Error::Payload);
        }
        Ok(())
    }
}

/// Build an inert owner-self-encrypted candidate using the supplied sender
/// seconds (no clock adjustment). Persist and retry this EXACT event; rebuilding
/// randomizes ciphertext and changes order. This function grants no authority
/// and does not publish. The producer must validate current bindings first.
pub fn build_event(owner: &Keys, payload: &Payload, created_at: u64) -> Result<Event, Error> {
    payload.validate()?;
    if payload.owner != owner.public_key() {
        return Err(Error::Scope);
    }
    let plaintext = Zeroizing::new(serde_json::to_string(payload).map_err(|_| Error::Payload)?);
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(Error::Payload);
    }
    let content = nip44::encrypt(
        owner.secret_key(),
        &owner.public_key(),
        plaintext.as_str(),
        nip44::Version::V2,
    )
    .map_err(|_| Error::Crypto)?;
    EventBuilder::new(Kind::Custom(KIND_PLACEMENT_INTENT as u16), content)
        .tags([Tag::parse(["L", NAMESPACE]).map_err(|_| Error::Envelope)?])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(owner)
        .map_err(|_| Error::Crypto)
}

/// Signed, scoped contribution plus separate request identity. Construction is
/// private so callers cannot accidentally substitute fields from another event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedIntent {
    payload: Payload,
    placement: PlacementIntent,
}

impl DecodedIntent {
    /// Authenticated request metadata; not permission to execute or replay it.
    pub fn payload(&self) -> &Payload {
        &self.payload
    }

    /// Bind M01 order/host/action to the same verified event, never arrival time.
    pub fn placement(&self) -> PlacementIntent {
        self.placement
    }
}

/// Verify hash AND signature before decrypting, strictly parse, then bind scope.
///
/// `community`, `agent`, and `authorized_hosts` must come from the captured
/// authenticated owner's authoritative bindings, NOT this event or a profile.
/// Hosts includes every authorized target whose history contributes to this
/// agent, not just the local executor: otherwise X cannot learn Start Y.
/// The caller must recheck authorization at effect boundaries. Historical
/// decoding is read-only projection, never command replay; no expiry/skew gate
/// is imposed on desired state. Errors are not evidence of absent intent.
pub fn decode_event(
    event: &Event,
    owner: &Keys,
    community: &str,
    agent: PublicKey,
    authorized_hosts: &[PublicKey],
) -> Result<DecodedIntent, Error> {
    if event.pubkey != owner.public_key() {
        return Err(Error::Scope);
    }
    if event.kind.as_u16() as u32 != KIND_PLACEMENT_INTENT
        || event.tags.len() != 1
        || !event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["L", NAMESPACE])
        || event.content.len() > MAX_CIPHERTEXT
        || !event.verify_id()
        || !event.verify_signature()
    {
        return Err(Error::Envelope);
    }
    let plaintext = Zeroizing::new(
        nip44::decrypt(owner.secret_key(), &owner.public_key(), &event.content)
            .map_err(|_| Error::Crypto)?,
    );
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(Error::Payload);
    }
    let payload: Payload = serde_json::from_str(&plaintext).map_err(|_| Error::Payload)?;
    payload.validate()?;
    if payload.owner != owner.public_key()
        || payload.community != community
        || payload.agent != agent
        || !authorized_hosts.contains(&payload.host)
    {
        return Err(Error::Scope);
    }
    let placement = PlacementIntent {
        order: EventOrder::from_event(event),
        host: payload.host,
        action: match payload.action {
            Action::Start => PlacementAction::Start,
            Action::Stop => PlacementAction::Stop,
        },
    };
    Ok(DecodedIntent { payload, placement })
}

#[cfg(test)]
mod tests;
