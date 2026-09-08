//! Owner-private display-only Desktop identity; never execution authority.
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, Tag};
use serde::{Deserialize, Serialize};

use crate::kind::KIND_DESKTOP_PROFILE;

/// Minimal encrypted profile. IDs are installation-local within an owner/community.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesktopProfile {
    /// Format version.
    pub v: u8,
    /// Canonical relay URL, bound inside the ciphertext.
    pub community: String,
    /// Opaque random coordinate, not an agent key.
    pub id: String,
    /// Generated display name, never a hostname.
    pub name: String,
}

/// Validate the public envelope without decrypting private content.
pub fn validate_envelope(event: &Event) -> Result<(), &'static str> {
    validate_private_desktop_envelope(event, KIND_DESKTOP_PROFILE)
}

pub(crate) fn validate_private_desktop_envelope(
    event: &Event,
    kind: u32,
) -> Result<(), &'static str> {
    let tags: Vec<_> = event.tags.iter().map(|tag| tag.as_slice()).collect();
    if event.kind.as_u16() as u32 != kind
        || event.created_at.as_secs() > 253_402_300_799
        || !(132..=2048).contains(&event.content.len())
        || tags.len() != 1
        || tags[0].len() != 2
        || tags[0][0] != "d"
        || !valid_id(&tags[0][1])
    {
        return Err("invalid Desktop profile envelope");
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl DesktopProfile {
    /// Construct a generated, non-identifying name for a random coordinate.
    pub fn new(community: String, id: String) -> Result<Self, &'static str> {
        if !valid_id(&id) || community.is_empty() || community.len() > 512 {
            return Err("invalid Desktop profile coordinate");
        }
        Ok(Self {
            v: 1,
            name: format!("Desktop {}", &id[..8]),
            community,
            id,
        })
    }

    /// Encrypt to the owner and sign the exact replaceable event.
    pub fn sign(&self, keys: &Keys) -> Result<Event, String> {
        let content = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            serde_json::to_string(self).map_err(|e| e.to_string())?,
            nip44::Version::V2,
        )
        .map_err(|e| e.to_string())?;
        EventBuilder::new(Kind::Custom(KIND_DESKTOP_PROFILE as u16), content)
            .tag(Tag::identifier(&self.id))
            .sign_with_keys(keys)
            .map_err(|e| e.to_string())
    }

    /// Verify hash/signature, owner, scope and exact payload before display.
    pub fn read(event: &Event, keys: &Keys, community: &str) -> Result<Self, String> {
        validate_envelope(event)?;
        event
            .verify()
            .map_err(|_| "invalid Desktop profile signature")?;
        if event.pubkey != keys.public_key() {
            return Err("foreign Desktop profile".into());
        }
        let plaintext = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
            .map_err(|_| "Desktop profile decryption failed")?;
        let profile: Self =
            serde_json::from_str(&plaintext).map_err(|_| "invalid Desktop profile")?;
        if profile
            != Self::new(
                community.to_owned(),
                event.tags.identifier().unwrap_or_default().to_owned(),
            )?
        {
            return Err("invalid Desktop profile payload or community".into());
        }
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_profile_roundtrip_and_untrusted_inputs() {
        let owner = Keys::generate();
        let profile = DesktopProfile::new("wss://one.example".into(), "a".repeat(32)).unwrap();
        let event = profile.sign(&owner).unwrap();
        assert_eq!(
            DesktopProfile::read(&event, &owner, &profile.community).unwrap(),
            profile
        );
        assert!(!event.content.contains(&profile.name));
        assert!(DesktopProfile::read(&event, &Keys::generate(), &profile.community).is_err());
        assert!(DesktopProfile::read(&event, &owner, "wss://two.example").is_err());
        let mut tampered = event.clone();
        tampered.content.push('x');
        assert!(DesktopProfile::read(&tampered, &owner, &profile.community).is_err());
        for field in ["v", "id", "name", "community", "extra"] {
            let mut payload = serde_json::to_value(&profile).unwrap();
            payload[field] = serde_json::json!("wrong");
            let encrypted = nip44::encrypt(
                owner.secret_key(),
                &owner.public_key(),
                payload.to_string(),
                nip44::Version::V2,
            )
            .unwrap();
            let invalid = EventBuilder::new(event.kind, encrypted)
                .tag(Tag::identifier(&profile.id))
                .sign_with_keys(&owner)
                .unwrap();
            assert!(
                DesktopProfile::read(&invalid, &owner, &profile.community).is_err(),
                "{field}"
            );
        }
        for tags in [
            vec![],
            vec![Tag::identifier("bad")],
            vec![Tag::identifier(&profile.id), Tag::identifier(&profile.id)],
        ] {
            let invalid = EventBuilder::new(event.kind, &event.content)
                .tags(tags)
                .sign_with_keys(&owner)
                .unwrap();
            assert!(validate_envelope(&invalid).is_err());
        }
        assert!(crate::kind::AUTHOR_ONLY_KINDS.contains(&KIND_DESKTOP_PROFILE));
        assert!(crate::kind::is_parameterized_replaceable(
            KIND_DESKTOP_PROFILE
        ));
    }
}
