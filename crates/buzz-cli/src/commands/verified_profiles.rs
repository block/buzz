//! NIP-FI verified profiles: relay-signed kind:30382 events that carry the
//! verified `username`, `display_name` and optional `full_name` of a pubkey.
//!
//! A community advertises them in NIP-11 (`federated_identity.public_profile`).
//! The only key trusted to sign them is the relay's NIP-11 `self` key, so a
//! kind:30382 signed by anyone else is ignored. Event and projection checks
//! mirror the Buzz mobile client (`verified_profile.dart`).

use std::collections::HashMap;

use nostr::Event;
use serde_json::{json, Value};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

use crate::client::BuzzClient;
use crate::commands::agents::normalize_relay_self_hex;
use crate::error::CliError;

/// Event kind of the relay-signed verified profile projection.
const KIND: u16 = buzz_core::kind::KIND_VERIFIED_PROFILE as u16;

/// Subjects per query, keeping each REQ inside relay filter limits.
const SUBJECTS_PER_QUERY: usize = 128;

/// Larger events (UTF-16 units of content and every tag element) are dropped
/// before any further work. A valid projection is well under 1 KiB.
const MAX_EVENT_SIZE: usize = 4096;

/// A verified name as the relay last published it. Historical: it records the
/// issuer's name-to-key assertion at `verified_at`, never current access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedProfile {
    pub display_name: String,
    pub username: String,
    pub full_name: Option<String>,
    pub issuer_label: String,
    pub verified_at: u64,
}

impl VerifiedProfile {
    /// The `verified` object `buzz users get` prints.
    pub(crate) fn to_json(&self) -> Value {
        let mut value = json!({
            "username": self.username,
            "display_name": self.display_name,
            "issuer_label": self.issuer_label,
            "verified_at": self.verified_at,
        });
        if let Some(full_name) = &self.full_name {
            value["full_name"] = json!(full_name);
        }
        value
    }
}

/// Read the relay key that signs verified profiles from a NIP-11 document.
///
/// `Ok(None)`: the community does not advertise verified profiles, so every
/// kind:30382 is ignored. A malformed advertisement is an error rather than
/// "no verified profiles", so an agent never reads a failed check as an
/// authoritative absence.
pub(crate) fn parse_discovery(info: &Value) -> Result<Option<String>, CliError> {
    let malformed = |what: &str| CliError::Other(format!("relay info document: {what}"));
    let Some(federated) = info.get("federated_identity") else {
        return Ok(None);
    };
    let federated = federated
        .as_object()
        .ok_or_else(|| malformed("malformed federated_identity"))?;
    let Some(profile) = federated.get("public_profile") else {
        return Ok(None);
    };
    let self_hex = info.get("self").and_then(Value::as_str);
    let (Some(profile), Some(self_hex)) = (profile.as_object(), self_hex) else {
        return Err(malformed("malformed public_profile discovery"));
    };
    if profile.get("kind").and_then(Value::as_u64) != Some(u64::from(KIND))
        || !profile
            .get("multiple_issuers")
            .is_some_and(serde_json::Value::is_boolean)
    {
        return Err(malformed("malformed public_profile discovery"));
    }
    normalize_relay_self_hex(self_hex).map(Some)
}

/// The key that signs this community's verified profiles, if it advertises any.
pub(crate) async fn discover_authority(client: &BuzzClient) -> Result<Option<String>, CliError> {
    let raw = client
        .get_public("/")
        .await
        .map_err(|e| CliError::Other(format!("failed to fetch relay info document: {e}")))?;
    let info: Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay info document is not valid JSON: {e}")))?;
    parse_discovery(&info)
}

/// Verified profiles for `pubkeys`, keyed by lowercase pubkey. Pubkeys without
/// a valid, active, relay-signed profile are absent.
pub(crate) async fn fetch_verified_profiles(
    client: &BuzzClient,
    authority: &str,
    pubkeys: &[String],
) -> Result<HashMap<String, VerifiedProfile>, CliError> {
    let mut subjects: Vec<String> = pubkeys.iter().map(|p| p.to_ascii_lowercase()).collect();
    subjects.sort();
    subjects.dedup();
    let mut verified = HashMap::new();
    for chunk in subjects.chunks(SUBJECTS_PER_QUERY) {
        let filter = json!({
            "kinds": [KIND],
            "authors": [authority],
            "#d": chunk,
            "limit": chunk.len(),
        });
        let raw = client.query(&filter).await?;
        let events: Vec<Value> = serde_json::from_str(&raw).map_err(|e| {
            CliError::Other(format!("failed to parse verified profile response: {e}"))
        })?;
        verified.extend(
            verified_profiles_from_events(&events, authority)
                .into_iter()
                .filter(|(subject, _)| chunk.contains(subject)),
        );
    }
    Ok(verified)
}

/// Every valid verified profile among `events`, keyed by subject pubkey.
///
/// Anything not signed by `authority`, not of kind 30382, oversized, or with
/// a bad id or signature is ignored. Per subject the newest event is the head,
/// as for any addressable event; a head that is not a valid active projection
/// (including a retraction) leaves the subject with no verified profile, so a
/// stale older name never resurfaces.
pub(crate) fn verified_profiles_from_events(
    events: &[Value],
    authority: &str,
) -> HashMap<String, VerifiedProfile> {
    let mut heads: HashMap<String, Event> = HashMap::new();
    for raw in events {
        let Ok(event) = serde_json::from_value::<Event>(raw.clone()) else {
            continue;
        };
        let Some(subject) = subject(&event, authority) else {
            continue;
        };
        if event.verify().is_err() {
            continue;
        }
        if heads
            .get(&subject)
            .is_none_or(|current| supersedes(&event, current))
        {
            heads.insert(subject, event);
        }
    }
    heads
        .into_iter()
        .filter_map(|(subject, event)| parse_projection(&event, &subject).map(|p| (subject, p)))
        .collect()
}

/// NIP-01 replaceable ordering: the later `created_at` wins; a tie goes to the
/// lower event id.
fn supersedes(candidate: &Event, current: &Event) -> bool {
    candidate.created_at > current.created_at
        || (candidate.created_at == current.created_at
            && candidate.id.to_hex() < current.id.to_hex())
}

fn is_lower_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn event_size(event: &Event) -> usize {
    let mut size = event.content.encode_utf16().count();
    for tag in event.tags.iter() {
        size += 1;
        for value in tag.as_slice() {
            size += value.encode_utf16().count() + 1;
        }
    }
    size
}

/// The subject a kind:30382 event addresses: its first `d` tag, when the event
/// is by `authority`, within the size bound, and the tag is a lowercase hex
/// pubkey. Projection shape is not checked here.
fn subject(event: &Event, authority: &str) -> Option<String> {
    if event.kind.as_u16() != KIND
        || event.pubkey.to_hex() != authority
        || event_size(event) > MAX_EVENT_SIZE
    {
        return None;
    }
    let d = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().map(String::as_str) == Some("d"))?;
    d.get(1).filter(|v| is_lower_hex64(v)).cloned()
}

/// Strictly validate the projection NIP-FI defines: empty content; exactly
/// seven tags (eight with `full_name`), each of two elements with no repeated
/// name; `d` and `p` equal; `active` equal to `true`; well-formed names,
/// username and issuer label; and a `verified_at` no later than `created_at`.
fn parse_projection(event: &Event, subject: &str) -> Option<VerifiedProfile> {
    if !event.content.is_empty() || !matches!(event.tags.len(), 7 | 8) {
        return None;
    }
    let mut tags: HashMap<&str, &str> = HashMap::new();
    for tag in event.tags.iter() {
        let [name, value] = tag.as_slice() else {
            return None;
        };
        if tags.insert(name, value).is_some() {
            return None;
        }
    }
    const REQUIRED: [&str; 7] = [
        "d",
        "p",
        "active",
        "display_name",
        "username",
        "issuer_label",
        "verified_at",
    ];
    let full_name = tags.get("full_name").copied();
    if !REQUIRED.iter().all(|name| tags.contains_key(name))
        || tags.len() != REQUIRED.len() + usize::from(full_name.is_some())
    {
        return None;
    }
    let (display_name, username, issuer_label) =
        (tags["display_name"], tags["username"], tags["issuer_label"]);
    let verified_at = tags["verified_at"];
    if tags["p"] != subject
        || tags["active"] != "true"
        || !full_name.is_none_or(valid_name)
        || !valid_name(display_name)
        || !valid_username(username)
        || !valid_name(issuer_label)
        || verified_at.is_empty()
        || !verified_at.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let verified_at: u64 = verified_at.parse().ok()?;
    if verified_at > event.created_at.as_secs() {
        return None;
    }
    Some(VerifiedProfile {
        display_name: display_name.to_owned(),
        username: username.to_owned(),
        full_name: full_name.map(str::to_owned),
        issuer_label: issuer_label.to_owned(),
        verified_at,
    })
}

/// A name or issuer label: 1 to 64 scalar values in NFC, no control, format,
/// surrogate, private-use, unassigned or line/paragraph separator characters,
/// and no whitespace at either end.
fn valid_name(value: &str) -> bool {
    use GeneralCategory::{
        Control, Format, LineSeparator, ParagraphSeparator, PrivateUse, Surrogate, Unassigned,
    };
    !value.is_empty()
        && value.chars().count() <= 64
        && value.trim() == value
        && unicode_normalization::is_nfc(value)
        && !value.chars().any(|c| {
            matches!(
                c.general_category(),
                Control
                    | Format
                    | Surrogate
                    | PrivateUse
                    | Unassigned
                    | LineSeparator
                    | ParagraphSeparator
            )
        })
}

/// A username: 1 to 64 characters of lowercase ASCII letters, digits, `.`, `_` or `-`.
fn valid_username(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    const AT: u64 = 1_800_000_000;

    fn tag_pairs<'a>(subject: &'a str, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        let mut tags = vec![
            ("d", subject),
            ("p", subject),
            ("active", "true"),
            ("display_name", "alice"),
            ("username", "alice"),
            ("issuer_label", "Example"),
            ("verified_at", "1799999999"),
        ];
        tags.extend_from_slice(extra);
        tags
    }

    fn sign(keys: &Keys, kind: u16, content: &str, tags: &[(&str, &str)], at: u64) -> Value {
        let tags: Vec<Tag> = tags
            .iter()
            .map(|(k, v)| Tag::parse([*k, *v]).unwrap())
            .collect();
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(keys)
            .unwrap();
        serde_json::to_value(event).unwrap()
    }

    #[test]
    fn discovery_reads_the_relay_key_only_where_advertised() {
        let key = "ab".repeat(32);
        let advertised = |profile: Value| json!({"self": key, "federated_identity": {"public_profile": profile}});
        let good = json!({"kind": 30382, "multiple_issuers": false, "searchable": true});
        assert_eq!(
            parse_discovery(&advertised(good)).unwrap(),
            Some(key.clone())
        );
        // Not advertised: absent or without `public_profile`.
        assert_eq!(parse_discovery(&json!({"self": key})).unwrap(), None);
        assert_eq!(
            parse_discovery(&json!({"self": key, "federated_identity": {}})).unwrap(),
            None
        );
        // Advertised but malformed is a failed check, not an absence.
        for bad in [
            advertised(json!(null)),
            advertised(json!({"kind": 30382})),
            advertised(json!({"kind": 1, "multiple_issuers": true})),
            advertised(json!({"kind": 30382, "multiple_issuers": "yes"})),
            json!({"federated_identity": {"public_profile": {"kind": 30382, "multiple_issuers": true}}}),
            json!({"self": "nothex", "federated_identity": {"public_profile": {"kind": 30382, "multiple_issuers": true}}}),
            json!({"self": key, "federated_identity": "on"}),
        ] {
            assert!(parse_discovery(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_relay_signed_active_projection_is_verified() {
        let relay = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let tags = tag_pairs(&subject, &[("full_name", "Alice Example")]);
        let event = sign(&relay, 30382, "", &tags, AT);
        let found = verified_profiles_from_events(&[event], &relay.public_key().to_hex());
        assert_eq!(
            found[&subject],
            VerifiedProfile {
                display_name: "alice".into(),
                username: "alice".into(),
                full_name: Some("Alice Example".into()),
                issuer_label: "Example".into(),
                verified_at: 1_799_999_999,
            }
        );
        assert_eq!(
            found[&subject].to_json(),
            json!({
                "username": "alice",
                "display_name": "alice",
                "full_name": "Alice Example",
                "issuer_label": "Example",
                "verified_at": 1_799_999_999,
            })
        );
    }

    #[test]
    fn full_name_is_omitted_when_absent() {
        let relay = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let event = sign(&relay, 30382, "", &tag_pairs(&subject, &[]), AT);
        let found = verified_profiles_from_events(&[event], &relay.public_key().to_hex());
        assert!(found[&subject].to_json().get("full_name").is_none());
    }

    #[test]
    fn an_event_signed_by_any_other_key_is_ignored() {
        let relay = Keys::generate();
        let impostor = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let forged = sign(&impostor, 30382, "", &tag_pairs(&subject, &[]), AT);
        assert!(verified_profiles_from_events(&[forged], &relay.public_key().to_hex()).is_empty());
    }

    #[test]
    fn a_tampered_or_wrong_kind_event_is_ignored() {
        let relay = Keys::generate();
        let authority = relay.public_key().to_hex();
        let subject = Keys::generate().public_key().to_hex();
        let mut tampered = sign(&relay, 30382, "", &tag_pairs(&subject, &[]), AT);
        tampered["tags"][4] = json!(["username", "mallory"]);
        let wrong_kind = sign(&relay, 30383, "", &tag_pairs(&subject, &[]), AT);
        assert!(verified_profiles_from_events(&[tampered, wrong_kind], &authority).is_empty());
    }

    #[test]
    fn malformed_projections_are_ignored() {
        let relay = Keys::generate();
        let authority = relay.public_key().to_hex();
        let subject = Keys::generate().public_key().to_hex();
        let other = Keys::generate().public_key().to_hex();
        let long = "a".repeat(65);
        let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
            ("content", vec![]),
            ("retracted", vec![("active", "false")]),
            ("p mismatch", vec![("p", &other)]),
            ("bad username", vec![("username", "Alice")]),
            ("empty username", vec![("username", "")]),
            ("padded name", vec![("display_name", " alice")]),
            ("control name", vec![("display_name", "al\nice")]),
            ("bidi name", vec![("full_name", "Al\u{202e}ice")]),
            ("non-NFC name", vec![("full_name", "e\u{301}")]),
            ("long name", vec![("display_name", &long)]),
            ("future verified_at", vec![("verified_at", "1800000001")]),
            ("signed verified_at", vec![("verified_at", "+5")]),
            ("extra tag", vec![("extra", "x")]),
        ];
        for (label, overrides) in cases {
            let mut tags = tag_pairs(&subject, &[]);
            for (key, value) in &overrides {
                match tags.iter_mut().find(|(k, _)| k == key) {
                    Some(existing) => existing.1 = value,
                    None => tags.push((key, value)),
                }
            }
            let content = if label == "content" { "hi" } else { "" };
            let event = sign(&relay, 30382, content, &tags, AT);
            assert!(
                verified_profiles_from_events(&[event], &authority).is_empty(),
                "{label}"
            );
        }
        let repeated = tag_pairs(&subject, &[("username", "bob")]);
        let event = sign(&relay, 30382, "", &repeated, AT);
        assert!(verified_profiles_from_events(&[event], &authority).is_empty());
    }

    #[test]
    fn the_newest_head_wins_and_a_retraction_clears_the_name() {
        let relay = Keys::generate();
        let authority = relay.public_key().to_hex();
        let subject = Keys::generate().public_key().to_hex();
        let old = sign(&relay, 30382, "", &tag_pairs(&subject, &[]), AT);
        let renamed = {
            let mut tags = tag_pairs(&subject, &[]);
            tags[4] = ("username", "alice2");
            sign(&relay, 30382, "", &tags, AT + 10)
        };
        let retracted = {
            let mut tags = tag_pairs(&subject, &[]);
            tags[2] = ("active", "false");
            sign(&relay, 30382, "", &tags, AT + 20)
        };
        let found = verified_profiles_from_events(&[old.clone(), renamed.clone()], &authority);
        assert_eq!(found[&subject].username, "alice2");
        let found = verified_profiles_from_events(&[renamed, retracted, old], &authority);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn subject_comes_from_the_first_d_tag_and_must_be_lowercase_hex() {
        let relay = Keys::generate();
        let authority = relay.public_key().to_hex();
        let upper = Keys::generate().public_key().to_hex().to_uppercase();
        let event = sign(&relay, 30382, "", &tag_pairs(&upper, &[]), AT);
        assert!(verified_profiles_from_events(&[event], &authority).is_empty());
    }
}
