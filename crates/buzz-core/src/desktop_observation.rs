//! Owner-private, advisory Desktop observations; never agent readiness or placement.
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, Tag};
use serde::{Deserialize, Serialize};

use crate::{desktop_profile::DesktopProfile, kind::KIND_DESKTOP_OBSERVATION};

/// A pulse for one local profile. The signed event timestamp is the observed time.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesktopObservation {
    /// Format version.
    pub v: u8,
    /// Canonical community, encrypted along with the coordinate.
    pub community: String,
    /// Stable Desktop profile coordinate, not an execution credential.
    pub id: String,
}

/// Validate the bounded public envelope without decrypting it.
pub fn validate_envelope(event: &Event) -> Result<(), &'static str> {
    crate::desktop_profile::validate_private_desktop_envelope(event, KIND_DESKTOP_OBSERVATION)
}

impl DesktopObservation {
    /// Observe a profile belonging to this local Desktop.
    pub fn new(profile: DesktopProfile) -> Self {
        Self {
            v: 1,
            community: profile.community,
            id: profile.id,
        }
    }

    /// Encrypt and sign a fresh observation, without rewriting the durable profile.
    pub fn sign(&self, keys: &Keys) -> Result<Event, String> {
        let content = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            serde_json::to_string(self).map_err(|e| e.to_string())?,
            nip44::Version::V2,
        )
        .map_err(|e| e.to_string())?;
        EventBuilder::new(Kind::Custom(KIND_DESKTOP_OBSERVATION as u16), content)
            .tag(Tag::identifier(&self.id))
            .sign_with_keys(keys)
            .map_err(|e| e.to_string())
    }

    /// Authenticate and decrypt an observation before displaying its timestamp.
    pub fn read(event: &Event, keys: &Keys, community: &str) -> Result<Self, String> {
        validate_envelope(event)?;
        event
            .verify()
            .map_err(|_| "invalid Desktop observation signature")?;
        if event.pubkey != keys.public_key() {
            return Err("foreign Desktop observation".into());
        }
        let plaintext = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
            .map_err(|_| "Desktop observation decryption failed")?;
        let observation: Self =
            serde_json::from_str(&plaintext).map_err(|_| "invalid Desktop observation")?;
        let expected = Self::new(DesktopProfile::new(
            community.to_owned(),
            event.tags.identifier().unwrap_or_default().to_owned(),
        )?);
        if observation != expected {
            return Err("Desktop observation scope mismatch".into());
        }
        Ok(observation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_is_private_scoped_and_distinct_from_profile() {
        let keys = Keys::generate();
        let profile = DesktopProfile::new("wss://one.example".into(), "a".repeat(32)).unwrap();
        let saved = profile.sign(&keys).unwrap();
        let observation = DesktopObservation::new(profile);
        let event = observation.sign(&keys).unwrap();
        assert_eq!(
            DesktopObservation::read(&event, &keys, &observation.community).unwrap(),
            observation
        );
        assert!(DesktopObservation::read(&event, &keys, "wss://two.example").is_err());
        assert!(
            DesktopObservation::read(&event, &Keys::generate(), &observation.community).is_err()
        );
        assert!(DesktopObservation::read(&saved, &keys, &observation.community).is_err());
        assert!(DesktopProfile::read(&event, &keys, &observation.community).is_err());
        let forged_author = EventBuilder::new(event.kind, &event.content)
            .tags(event.tags.clone())
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(DesktopObservation::read(&forged_author, &keys, &observation.community).is_err());
        let mut tampered = event.clone();
        tampered.created_at = nostr::Timestamp::from(1);
        assert!(DesktopObservation::read(&tampered, &keys, &observation.community).is_err());
        assert!(crate::kind::AUTHOR_ONLY_KINDS.contains(&KIND_DESKTOP_OBSERVATION));
        for field in ["v", "community", "id", "extra"] {
            let mut payload = serde_json::to_value(&observation).unwrap();
            payload[field] = serde_json::json!("invalid");
            let ciphertext = nip44::encrypt(
                keys.secret_key(),
                &keys.public_key(),
                payload.to_string(),
                nip44::Version::V2,
            )
            .unwrap();
            let invalid = EventBuilder::new(event.kind, ciphertext)
                .tags(event.tags.clone())
                .sign_with_keys(&keys)
                .unwrap();
            assert!(
                DesktopObservation::read(&invalid, &keys, &observation.community).is_err(),
                "{field}"
            );
        }
    }
}
