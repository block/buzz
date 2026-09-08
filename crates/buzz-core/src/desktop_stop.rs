//! Immutable, owner-to-self Desktop Stop messages. Profiles are not authority.
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};

use crate::kind::{KIND_DESKTOP_STOP, KIND_DESKTOP_STOP_RESULT};

/// One agent on one Desktop in one community; never a caller-selected process.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StopTarget {
    /// Schema version. Old exact-run commands are not accepted here.
    pub v: u8,
    /// Canonical community WebSocket URL.
    pub community: String,
    /// Installation coordinate from the private Desktop inventory.
    pub desktop: String,
    /// Agent public key. The receiver independently verifies local ownership.
    pub agent: String,
}

/// Ordinary Desktop outcome, not a stronger process-termination certificate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopOutcome {
    /// Ordinary Stop returned success.
    Stopped,
    /// Ordinary Stop returned an error. No automatic retry of the effect.
    Failed,
    /// Interrupted, stale or evicted request; never inferred success.
    Unknown,
}

/// Correlates exactly one immutable request with its Desktop's result.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StopResult {
    /// Original target, not mutable current routing.
    pub target: StopTarget,
    /// Signed request event ID.
    pub request: String,
    /// No diagnostic paths, credentials or process details on the wire.
    pub outcome: StopOutcome,
}

pub(crate) fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Check public shape before storage, without decrypting content.
pub fn validate_envelope(event: &Event) -> Result<(), &'static str> {
    let kind = event.kind.as_u16() as u32;
    let tags: Vec<_> = event.tags.iter().map(|t| t.as_slice()).collect();
    let result = kind == KIND_DESKTOP_STOP_RESULT;
    if !matches!(kind, KIND_DESKTOP_STOP | KIND_DESKTOP_STOP_RESULT)
        || !(132..=4096).contains(&event.content.len())
        || tags.len() != if result { 2 } else { 1 }
        || tags[0].len() != 2
        || tags[0][0] != "d"
        || !hex(&tags[0][1], 32)
        || (result && (tags[1].len() != 2 || tags[1][0] != "e" || !hex(&tags[1][1], 64)))
    {
        return Err("invalid Desktop Stop envelope");
    }
    Ok(())
}

pub(crate) fn sign<T: Serialize>(
    value: &T,
    keys: &Keys,
    kind: u32,
    tags: Vec<Tag>,
) -> Result<Event, String> {
    let ciphertext = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        serde_json::to_string(value).map_err(|e| e.to_string())?,
        nip44::Version::V2,
    )
    .map_err(|e| e.to_string())?;
    EventBuilder::new(Kind::Custom(kind as u16), ciphertext)
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| e.to_string())
}

pub(crate) fn read<T: serde::de::DeserializeOwned>(
    event: &Event,
    keys: &Keys,
    kind: u32,
) -> Result<T, String> {
    if matches!(kind, KIND_DESKTOP_STOP | KIND_DESKTOP_STOP_RESULT) {
        validate_envelope(event)?;
    } else {
        crate::desktop_lifecycle::validate_envelope(event)?;
    }
    event
        .verify()
        .map_err(|_| "invalid Desktop Stop signature")?;
    if event.pubkey != keys.public_key() || event.kind.as_u16() as u32 != kind {
        return Err("foreign Desktop Stop message".into());
    }
    let plaintext = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
        .map_err(|_| "Desktop Stop decryption failed")?;
    serde_json::from_str(&plaintext).map_err(|_| "invalid Desktop Stop payload".into())
}

impl StopTarget {
    /// Validate the decrypted target against the captured community.
    pub fn validate(&self, community: &str) -> Result<(), &'static str> {
        if self.v != 1
            || self.community != community
            || community.is_empty()
            || community.len() > 512
            || !hex(&self.desktop, 32)
            || !hex(&self.agent, 64)
            || PublicKey::from_hex(&self.agent).is_err()
        {
            return Err("invalid Desktop Stop target");
        }
        Ok(())
    }

    /// Produce a new immutable Stop. Transport retries must reuse this event.
    pub fn sign(&self, keys: &Keys) -> Result<Event, String> {
        self.validate(&self.community)?;
        sign(
            self,
            keys,
            KIND_DESKTOP_STOP,
            vec![Tag::identifier(&self.desktop)],
        )
    }

    /// Authenticate, decrypt and bind a Stop to its signed host coordinate.
    pub fn read(event: &Event, keys: &Keys, community: &str) -> Result<Self, String> {
        let target: Self = read(event, keys, KIND_DESKTOP_STOP)?;
        target.validate(community)?;
        if event.tags.identifier() != Some(target.desktop.as_str()) {
            return Err("Desktop Stop routing mismatch".into());
        }
        Ok(target)
    }
}

impl StopResult {
    /// Sign the saved ordinary Stop result without exposing local diagnostics.
    pub fn sign(&self, keys: &Keys) -> Result<Event, String> {
        self.target.validate(&self.target.community)?;
        if !hex(&self.request, 64) {
            return Err("invalid Stop request ID".into());
        }
        sign(
            self,
            keys,
            KIND_DESKTOP_STOP_RESULT,
            vec![
                Tag::identifier(&self.target.desktop),
                Tag::parse(["e", &self.request]).map_err(|e| e.to_string())?,
            ],
        )
    }

    /// Check all correlation fields against the original authenticated request.
    pub fn read(
        event: &Event,
        keys: &Keys,
        request: &Event,
        community: &str,
    ) -> Result<Self, String> {
        let target = StopTarget::read(request, keys, community)?;
        let result: Self = read(event, keys, KIND_DESKTOP_STOP_RESULT)?;
        if result.target != target
            || result.request != request.id.to_hex()
            || event.tags.identifier() != Some(target.desktop.as_str())
            || event.tags.iter().nth(1).and_then(|t| t.content()) != Some(result.request.as_str())
        {
            return Err("Desktop Stop result correlation mismatch".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_immutable_stop_and_exact_result_correlation() {
        let keys = Keys::generate();
        let target = StopTarget {
            v: 1,
            community: "wss://one.example".into(),
            desktop: "a".repeat(32),
            agent: Keys::generate().public_key().to_hex(),
        };
        let request = target.sign(&keys).unwrap();
        assert_eq!(
            StopTarget::read(&request, &keys, &target.community).unwrap(),
            target
        );
        assert!(!request.content.contains(&target.agent));
        assert!(StopTarget::read(&request, &Keys::generate(), &target.community).is_err());
        assert!(StopTarget::read(&request, &keys, "wss://other.example").is_err());
        let result = StopResult {
            target: target.clone(),
            request: request.id.to_hex(),
            outcome: StopOutcome::Stopped,
        }
        .sign(&keys)
        .unwrap();
        assert_eq!(
            StopResult::read(&result, &keys, &request, &target.community)
                .unwrap()
                .outcome,
            StopOutcome::Stopped
        );
        let another = target.sign(&keys).unwrap();
        assert!(StopResult::read(&result, &keys, &another, &target.community).is_err());
        let mut tampered = request.clone();
        tampered.content.push('x');
        assert!(StopTarget::read(&tampered, &keys, &target.community).is_err());
        for kind in [KIND_DESKTOP_STOP, KIND_DESKTOP_STOP_RESULT] {
            assert!(crate::kind::AUTHOR_ONLY_KINDS.contains(&kind));
            assert!(!crate::kind::is_parameterized_replaceable(kind));
        }
    }
}
