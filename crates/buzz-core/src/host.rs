//! Versioned, owner-private host events. Kind 50000 is append-only, not NIP-33.
use base64::Engine;
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};

use crate::kind::KIND_HOST;

/// Namespace for host labels (NIP-32-shaped tags).
pub const NAMESPACE: &str = "buzz.host.v1";
/// Maximum lifetime of a host report, independent of agent presence.
pub const REPORT_TTL: u64 = 180;

/// Validated public routing envelope. Machine metadata is never in tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Owner allowed to read the event.
    pub owner: PublicKey,
    /// Stable host key (also the host identifier).
    pub host: PublicKey,
    /// Registration or report.
    pub label: String,
    /// Owner-signed registration referenced by a report.
    pub registration: Option<String>,
    /// Readiness deadline; not a NIP-40 deletion deadline.
    pub valid_until: Option<u64>,
}

fn one<'a>(event: &'a Event, key: &str, size: usize) -> Result<&'a [String], String> {
    let mut tags = event.tags.iter().filter(|t| t.as_slice()[0] == key);
    let tag = tags.next().ok_or_else(|| format!("missing {key} tag"))?;
    if tags.next().is_some() || tag.as_slice().len() != size {
        return Err(format!("invalid {key} tag cardinality"));
    }
    Ok(tag.as_slice())
}

fn pubkey(value: &str) -> Result<PublicKey, String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("expected lowercase hex pubkey".into());
    }
    PublicKey::from_hex(value).map_err(|e| e.to_string())
}

/// Validate routing and signatures without decrypting private machine details.
pub fn validate(event: &Event) -> Result<Envelope, String> {
    if event.kind.as_u16() as u32 != KIND_HOST {
        return Err("not a host event".into());
    }
    crate::verify_event(event).map_err(|e| e.to_string())?;
    if one(event, "L", 2)?[1] != NAMESPACE {
        return Err("unknown host namespace".into());
    }
    let l = one(event, "l", 3)?;
    if l[2] != NAMESPACE || !matches!(l[1].as_str(), "registration" | "report" | "profile") {
        return Err("unknown host label".into());
    }
    let owner = pubkey(&one(event, "p", 2)?[1])?;
    let host = pubkey(&one(event, "x", 2)?[1])?;
    let report = l[1] != "registration";
    let allowed = if report {
        if l[1] == "report" {
            &["L", "l", "p", "x", "e", "valid_until"][..]
        } else {
            &["L", "l", "p", "x", "e"][..]
        }
    } else {
        &["L", "l", "p", "x"][..]
    };
    if event
        .tags
        .iter()
        .any(|t| !allowed.contains(&t.as_slice()[0].as_str()))
    {
        return Err("unexpected host tag".into());
    }
    if event.pubkey != if report { host } else { owner } {
        return Err("host event signer does not match its role".into());
    }
    // Reject cleartext and malformed NIP-44 envelopes before storage. The owner
    // alone can authenticate/decrypt the encrypted bytes.
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&event.content)
        .map_err(|_| "invalid encrypted host content")?;
    if !(99..=36_000).contains(&bytes.len()) || bytes.first() != Some(&2) {
        return Err("invalid encrypted host content".into());
    }
    let (registration, valid_until) = if report {
        let id = one(event, "e", 2)?[1].clone();
        if id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid registration reference".into());
        }
        let until = if l[1] == "report" {
            let until = one(event, "valid_until", 2)?[1]
                .parse::<u64>()
                .map_err(|_| "invalid report deadline")?;
            let ts = event.created_at.as_secs();
            if until <= ts || until > ts.saturating_add(REPORT_TTL) {
                return Err("report lifetime exceeds limit".into());
            }
            Some(until)
        } else {
            None
        };
        (Some(id), until)
    } else {
        (None, None)
    };
    Ok(Envelope {
        owner,
        host,
        label: l[1].clone(),
        registration,
        valid_until,
    })
}

/// Small allowlisted runtime projection; never commands, paths or environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    /// Catalog identifier.
    pub id: String,
    /// Human-facing catalog label.
    pub label: String,
    /// Catalog availability, not an assertion that a launch will succeed.
    pub availability: String,
    /// Cached catalog authentication observation.
    pub auth_status: String,
}

/// Opaque reference to an already-provisioned destination configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionedAgent {
    /// Agent public identity, never its key.
    pub agent: String,
    /// Destination Rust catalog runtime ID.
    pub runtime: String,
    /// Digest rechecked at the actual spawn boundary.
    pub revision: String,
}

/// Encrypted machine report. Registration alone does not enable remote launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Protocol version.
    pub v: u8,
    /// OS-reported machine name.
    pub name: String,
    /// Operating system.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Reporting launcher version.
    pub launcher_version: String,
    /// Observed catalog, with sensitive fields omitted.
    pub runtimes: Vec<Runtime>,
    /// False until the host implements launch request handling.
    pub accepts_start: bool,
    /// Owner-private ready configurations; absence requires destination setup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provisioned: Vec<ProvisionedAgent>,
}

impl Report {
    /// Check bounded data before encryption and after authenticated decryption.
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.v, 1..=3)
            || (self.v != 3 && (self.accepts_start || !self.provisioned.is_empty()))
            || self.runtimes.len() > 128
            || self.provisioned.len() > 256
        {
            return Err("unsupported host report".into());
        }
        let mut agents = std::collections::HashSet::new();
        for config in &self.provisioned {
            if !crate::host_execution::hex_id(&config.agent, 64)
                || !crate::host_execution::hex_id(&config.revision, 64)
                || !agents.insert(&config.agent)
                || !self.runtimes.iter().any(|r| {
                    r.id == config.runtime
                        && r.availability == "available"
                        && matches!(r.auth_status.as_str(), "logged_in" | "not_applicable")
                })
            {
                return Err("invalid provisioned configuration".into());
            }
        }
        let strings = [&self.name, &self.os, &self.arch, &self.launcher_version];
        if strings
            .into_iter()
            .chain(
                self.runtimes
                    .iter()
                    .flat_map(|r| [&r.id, &r.label, &r.availability, &r.auth_status]),
            )
            .any(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
        {
            return Err("invalid host report text".into());
        }
        Ok(())
    }
}

fn build(
    signer: &Keys,
    owner: PublicKey,
    host: PublicKey,
    label: &str,
    plaintext: &str,
    extra: Vec<Vec<String>>,
    now: u64,
) -> Result<Event, String> {
    let content = nip44::encrypt(signer.secret_key(), &owner, plaintext, nip44::Version::V2)
        .map_err(|e| e.to_string())?;
    let mut tags = vec![
        vec!["L".into(), NAMESPACE.into()],
        vec!["l".into(), label.into(), NAMESPACE.into()],
        vec!["p".into(), owner.to_hex()],
        vec!["x".into(), host.to_hex()],
    ];
    tags.extend(extra);
    let tags = tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let event = EventBuilder::new(Kind::Custom(KIND_HOST as u16), content)
        .allow_self_tagging()
        .tags(tags)
        .custom_created_at(Timestamp::from(now))
        .sign_with_keys(signer)
        .map_err(|e| e.to_string())?;
    validate(&event)?;
    Ok(event)
}

/// Build an owner-approved registration. Reuse an existing registration on restart.
pub fn registration(owner: &Keys, host: PublicKey, now: u64) -> Result<Event, String> {
    build(
        owner,
        owner.public_key(),
        host,
        "registration",
        r#"{"v":1}"#,
        vec![],
        now,
    )
}

/// Build a report signed by the host and encrypted to the registered owner.
pub fn report(
    host: &Keys,
    registration: &Event,
    payload: &Report,
    now: u64,
) -> Result<Event, String> {
    let env = validate(registration)?;
    if env.label != "registration" || env.host != host.public_key() {
        return Err("registration does not authorize this host".into());
    }
    payload.validate()?;
    build(
        host,
        env.owner,
        env.host,
        "report",
        &serde_json::to_string(payload).map_err(|e| e.to_string())?,
        vec![
            vec!["e".into(), registration.id.to_hex()],
            vec![
                "valid_until".into(),
                now.saturating_add(REPORT_TTL).to_string(),
            ],
        ],
        now,
    )
}

/// Durable change-only profile, signed by the host and encrypted to its owner.
pub fn profile(
    host: &Keys,
    registration: &Event,
    payload: &Report,
    now: u64,
) -> Result<Event, String> {
    let env = validate(registration)?;
    if env.label != "registration" || env.host != host.public_key() || !matches!(payload.v, 2 | 3) {
        return Err("invalid host profile binding or version".into());
    }
    payload.validate()?;
    build(
        host,
        env.owner,
        env.host,
        "profile",
        &serde_json::to_string(payload).map_err(|e| e.to_string())?,
        vec![vec!["e".into(), registration.id.to_hex()]],
        now,
    )
}

/// Verify both signatures and their binding before decrypting machine data.
pub fn decrypt_report(
    owner: &Keys,
    registration: &Event,
    report: &Event,
) -> Result<Report, String> {
    let reg = validate(registration)?;
    let env = validate(report)?;
    if reg.label != "registration"
        || !matches!(env.label.as_str(), "report" | "profile")
        || env.owner != owner.public_key()
        || reg.owner != env.owner
        || reg.host != env.host
        || env.registration.as_deref() != Some(registration.id.to_hex().as_str())
    {
        return Err("host report binding mismatch".into());
    }
    let text = nip44::decrypt(owner.secret_key(), &env.host, &report.content)
        .map_err(|e| e.to_string())?;
    let result: Report = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    result.validate()?;
    if (env.label == "profile") != matches!(result.v, 2 | 3) {
        return Err("host payload/envelope version mismatch".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload() -> Report {
        Report {
            v: 1,
            name: "Actual machine".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            launcher_version: "test".into(),
            runtimes: vec![],
            accepts_start: false,
            provisioned: vec![],
        }
    }
    #[test]
    fn round_trip_and_foreign_owner_rejected() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let reg = registration(&owner, host.public_key(), 100).unwrap();
        let rep = report(&host, &reg, &payload(), 101).unwrap();
        assert_eq!(
            decrypt_report(&owner, &reg, &rep).unwrap().name,
            "Actual machine"
        );
        assert!(decrypt_report(&Keys::generate(), &reg, &rep).is_err());
        assert_eq!(validate(&rep).unwrap().valid_until, Some(281));
        assert!(!rep.content.contains("Actual machine"));
        for event in [&reg, &rep] {
            assert!(crate::filter::reader_authorized_for_event(
                event,
                &owner.public_key().to_hex()
            ));
            assert!(!crate::filter::reader_authorized_for_event(
                event,
                &Keys::generate().public_key().to_hex()
            ));
            assert!(!crate::filter::reader_authorized_for_event(
                event,
                &host.public_key().to_hex()
            ));
        }
        assert!(!crate::kind::is_parameterized_replaceable(KIND_HOST));
    }
    #[test]
    fn mismatched_registration_and_signature_rejected() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let reg = registration(&owner, host.public_key(), 100).unwrap();
        assert!(report(&Keys::generate(), &reg, &payload(), 101).is_err());
        let rep = report(&host, &reg, &payload(), 101).unwrap();
        let other = registration(&owner, host.public_key(), 99).unwrap();
        assert!(decrypt_report(&owner, &other, &rep).is_err());
        let mut tampered = rep.clone();
        tampered.content.push('A');
        assert!(validate(&tampered).is_err());
    }
    #[test]
    fn malformed_envelopes_rejected_even_when_signed() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let reg = registration(&owner, host.public_key(), 100).unwrap();
        for extra in [
            vec!["p", &owner.public_key().to_hex()],
            vec!["h", "channel"],
            vec!["L", "unknown"],
        ] {
            let mut tags: Vec<Tag> = reg.tags.iter().cloned().collect();
            tags.push(Tag::parse(extra).unwrap());
            let bad = EventBuilder::new(reg.kind, reg.content.clone())
                .allow_self_tagging()
                .tags(tags)
                .sign_with_keys(&owner)
                .unwrap();
            assert!(validate(&bad).is_err());
        }
        let mut p = payload();
        p.accepts_start = true;
        assert!(report(&host, &reg, &p, 101).is_err());
    }
}
