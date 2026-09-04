//! Bounded, owner-private runtime facts, not signing access or agent readiness.
use crate::{desktop_profile::DesktopProfile, kind::KIND_DESKTOP_CAPABILITIES};
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, Tag};
use serde::{Deserialize, Serialize};

/// Allowlisted projection of a built-in runtime; never catalog paths or auth data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFact {
    /// Built-in catalog identifier.
    pub id: String,
    /// Discovery's installation/adapter availability, not authentication.
    pub availability: String,
    /// Whether a separate vendor CLI is required.
    pub requires_external_cli: bool,
    /// Spawn policy cap; None means no configured cap, not infinite capacity.
    pub max_parallelism: Option<u32>,
}

/// Facts at the signed event time, changed only when the projection changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesktopCapabilities {
    /// Format version.
    pub v: u8,
    /// Encrypted canonical community.
    pub community: String,
    /// Local Desktop coordinate.
    pub id: String,
    /// Sorted, unique built-in runtime facts.
    pub runtimes: Vec<RuntimeFact>,
}

/// Validate the bounded public envelope without decrypting it.
pub fn validate_envelope(event: &Event) -> Result<(), &'static str> {
    crate::desktop_profile::validate_private_desktop_envelope(event, KIND_DESKTOP_CAPABILITIES)
}

impl DesktopCapabilities {
    /// Project onto the persisted Desktop coordinate, not a caller-selected host.
    pub fn new(profile: DesktopProfile, mut runtimes: Vec<RuntimeFact>) -> Self {
        runtimes.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            v: 1,
            community: profile.community,
            id: profile.id,
            runtimes,
        }
    }

    fn validate(&self) -> Result<(), String> {
        DesktopProfile::new(self.community.clone(), self.id.clone())?;
        if self.v != 1
            || self.runtimes.len() > 8
            || self.runtimes.windows(2).any(|r| r[0].id >= r[1].id)
            || self.runtimes.iter().any(|r| {
                r.id.is_empty()
                    || r.id.len() > 32
                    || !r.id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    || !matches!(
                        r.availability.as_str(),
                        "available"
                            | "adapter_missing"
                            | "adapter_outdated"
                            | "cli_missing"
                            | "not_installed"
                    )
                    || r.max_parallelism == Some(0)
            })
        {
            return Err("invalid Desktop runtime facts".into());
        }
        Ok(())
    }

    /// Encrypt/sign once, then persist these exact bytes for retries.
    pub fn sign(&self, keys: &Keys) -> Result<Event, String> {
        self.validate()?;
        let content = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            serde_json::to_string(self).map_err(|e| e.to_string())?,
            nip44::Version::V2,
        )
        .map_err(|e| e.to_string())?;
        let event = EventBuilder::new(Kind::Custom(KIND_DESKTOP_CAPABILITIES as u16), content)
            .tag(Tag::identifier(&self.id))
            .sign_with_keys(keys)
            .map_err(|e| e.to_string())?;
        validate_envelope(&event)?;
        Ok(event)
    }

    /// Bounded history/live merge: newest signed time, lower event ID on ties.
    pub fn read_latest(
        mut events: Vec<Event>,
        keys: &Keys,
        community: &str,
    ) -> Result<Vec<(Self, u64)>, String> {
        if events.len() > 100 {
            return Err("too many Desktop reports".into());
        }
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        let mut seen = std::collections::HashSet::new();
        let mut rows = Vec::new();
        for event in events {
            let report = Self::read(&event, keys, community)?;
            if seen.insert(report.id.clone()) {
                rows.push((report, event.created_at.as_secs()));
            }
        }
        Ok(rows)
    }

    /// Authenticate, decrypt and scope-check before exposing any fact.
    pub fn read(event: &Event, keys: &Keys, community: &str) -> Result<Self, String> {
        validate_envelope(event)?;
        event
            .verify()
            .map_err(|_| "invalid Desktop report signature")?;
        if event.pubkey != keys.public_key() {
            return Err("foreign Desktop report".into());
        }
        let plaintext = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
            .map_err(|_| "Desktop report decryption failed")?;
        let report: Self =
            serde_json::from_str(&plaintext).map_err(|_| "invalid Desktop report")?;
        report.validate()?;
        if report.community != community || Some(report.id.as_str()) != event.tags.identifier() {
            return Err("Desktop report scope mismatch".into());
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_scoped_bounded_facts() {
        let keys = Keys::generate();
        let mut report = DesktopCapabilities::new(
            DesktopProfile::new("wss://one.example".into(), "a".repeat(32)).unwrap(),
            vec![],
        );
        let event = report.sign(&keys).unwrap();
        assert_eq!(
            DesktopCapabilities::read(&event, &keys, &report.community).unwrap(),
            report
        );
        assert!(DesktopCapabilities::read(&event, &keys, "wss://two.example").is_err());
        assert!(DesktopCapabilities::read(&event, &Keys::generate(), &report.community).is_err());
        let mut payload = serde_json::to_value(&report).unwrap();
        payload["auth"] = serde_json::json!("must not appear");
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
        assert!(DesktopCapabilities::read(&invalid, &keys, &report.community).is_err());
        let mut tampered = event;
        tampered.created_at = nostr::Timestamp::from(1);
        assert!(DesktopCapabilities::read(&tampered, &keys, &report.community).is_err());
        report.runtimes.push(RuntimeFact {
            id: "/private/path".into(),
            availability: "available".into(),
            requires_external_cli: false,
            max_parallelism: None,
        });
        assert!(report.sign(&keys).is_err());
        assert!(crate::kind::AUTHOR_ONLY_KINDS.contains(&KIND_DESKTOP_CAPABILITIES));
        report.runtimes[0].id = "goose".into();
        let old = report.sign(&keys).unwrap();
        report.runtimes[0].availability = "cli_missing".into();
        let new = report.sign(&keys).unwrap();
        let signed = |event: &Event, time| {
            EventBuilder::new(event.kind, &event.content)
                .tags(event.tags.clone())
                .custom_created_at(nostr::Timestamp::from(time))
                .sign_with_keys(&keys)
                .unwrap()
        };
        let a = signed(&old, 20);
        let b = signed(&new, 20);
        let winner = if a.id < b.id { &a } else { &b };
        let expected = DesktopCapabilities::read(winner, &keys, &report.community).unwrap();
        for events in [vec![signed(&old, 10), a.clone(), b.clone()], vec![b, a]] {
            assert_eq!(
                DesktopCapabilities::read_latest(events, &keys, &report.community).unwrap(),
                vec![(expected.clone(), 20)]
            );
        }
        assert!(
            DesktopCapabilities::read_latest(vec![old; 101], &keys, &report.community).is_err()
        );
    }
}
