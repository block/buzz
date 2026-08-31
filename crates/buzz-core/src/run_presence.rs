//! Bounded per-run presence. Location is a launcher assertion, not launch authority.
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};

/// Presence lifetime, shared by publishers, relay and consumers.
pub const LEASE_SECONDS: u64 = 180;
/// A single live launcher run; offline records remain as ordering tombstones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunPresence {
    /// Random process-generation identifier (32 lowercase hex characters).
    pub run: String,
    /// Monotonically increasing pulse sequence within this generation.
    pub seq: u64,
    /// Bare online, away or offline status.
    pub status: String,
    /// Actual publisher deadline, never renewed by reading a snapshot.
    pub expires_at: u64,
    /// Deliberately public host reference and display alias, if supplied.
    pub location: Option<Location>,
    /// Owner-signed host binding for owner-transported host pulses only.
    pub registration: Option<String>,
}
/// Minimal public placement descriptor; never includes private capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Location {
    /// Stable launcher installation key.
    pub host: String,
    /// Public display alias, not an implicitly exported OS hostname.
    pub label: String,
}
fn hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Location {
    /// Reject unbounded, misleading control text and malformed identifiers.
    pub fn validate(&self) -> Result<(), String> {
        if !hex(&self.host, 64)
            || PublicKey::from_hex(&self.host).is_err()
            || self.label.trim().is_empty()
            || self.label.len() > 80
            || self.label.chars().any(|c| {
                c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
        {
            return Err("invalid public host location".into());
        }
        Ok(())
    }
}
/// Parse the upgraded wire format; legacy no-run presence stays on its old path.
/// Callers must separately verify event signature and transport authority.
pub fn parse_run(event: &Event, now: u64) -> Result<Option<RunPresence>, String> {
    if !event.tags.iter().any(|t| {
        matches!(
            t.as_slice()[0].as_str(),
            "run" | "seq" | "host" | "host_registration"
        )
    }) {
        return Ok(None);
    }
    if event.kind.as_u16() as u32 != crate::kind::KIND_PRESENCE_UPDATE
        || !matches!(event.content.as_str(), "online" | "away" | "offline")
    {
        return Err("invalid run presence status".into());
    }
    let ts = event.created_at.as_secs();
    if ts > now.saturating_add(30) || ts.saturating_add(LEASE_SECONDS) <= now {
        return Err("run presence timestamp outside lease window".into());
    }
    let mut run = None;
    let mut seq = None;
    let mut location = None;
    let mut registration = None;
    for tag in event.tags.iter() {
        let t = tag.as_slice();
        match t[0].as_str() {
            "run" if t.len() == 2 && run.is_none() && hex(&t[1], 32) => run = Some(t[1].clone()),
            "seq" if t.len() == 2 && seq.is_none() => {
                let n = t[1]
                    .parse::<u64>()
                    .map_err(|_| "invalid presence sequence")?;
                // Lua compares exactly representable integers.
                if n > 9_007_199_254_740_991 {
                    return Err("presence sequence too large".into());
                }
                seq = Some(n);
            }
            "host" if t.len() == 3 && location.is_none() => {
                let l = Location {
                    host: t[1].clone(),
                    label: t[2].clone(),
                };
                l.validate()?;
                location = Some(l);
            }
            "host_registration" if t.len() == 2 && registration.is_none() && hex(&t[1], 64) => {
                registration = Some(t[1].clone())
            }
            _ => return Err("invalid run presence tags".into()),
        }
    }
    Ok(Some(RunPresence {
        run: run.ok_or("missing presence run")?,
        seq: seq.ok_or("missing presence sequence")?,
        status: event.content.clone(),
        expires_at: ts
            .saturating_add(LEASE_SECONDS)
            .min(now.saturating_add(LEASE_SECONDS)),
        location,
        registration,
    }))
}
/// Sign one pulse with a process-stable run and increasing sequence.
pub fn pulse(
    keys: &Keys,
    run: &str,
    seq: u64,
    status: &str,
    location: Option<&Location>,
    registration: Option<&str>,
    now: u64,
) -> Result<Event, String> {
    let mut tags = vec![
        vec!["run".into(), run.into()],
        vec!["seq".into(), seq.to_string()],
    ];
    if let Some(l) = location {
        tags.push(vec!["host".into(), l.host.clone(), l.label.clone()]);
    }
    if let Some(id) = registration {
        tags.push(vec!["host_registration".into(), id.into()]);
    }
    let tags = tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let event = EventBuilder::new(
        Kind::Custom(crate::kind::KIND_PRESENCE_UPDATE as u16),
        status,
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(now))
    .sign_with_keys(keys)
    .map_err(|e| e.to_string())?;
    parse_run(&event, now)?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn run_wire_is_bounded_and_legacy_is_separate() {
        let keys = Keys::generate();
        let location = Location {
            host: Keys::generate().public_key().to_hex(),
            label: "Workshop".into(),
        };
        let event = pulse(
            &keys,
            &"a".repeat(32),
            1,
            "online",
            Some(&location),
            None,
            100,
        )
        .unwrap();
        let parsed = parse_run(&event, 110).unwrap().unwrap();
        assert_eq!(parsed.expires_at, 280);
        assert_eq!(parsed.location, Some(location));
        assert!(parse_run(&event, 280).is_err());
        assert!(parse_run(&event, 69).is_err());
        assert!(pulse(&keys, "bad", 0, "online", None, None, 100).is_err());
        assert!(pulse(&keys, &"a".repeat(32), 0, "ready", None, None, 100).is_err());
        let legacy = EventBuilder::new(Kind::Custom(20001), "online")
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(parse_run(&legacy, 100).unwrap(), None);
    }
}
