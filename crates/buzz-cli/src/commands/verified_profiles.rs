//! NIP-FI verified profiles: relay-signed kind:30382 events that carry the
//! verified `username`, `display_name` and optional `full_name` of a pubkey.
//!
//! A community advertises them in NIP-11 (`federated_identity.public_profile`).
//! The only key trusted to sign them is the relay's NIP-11 `self` key, so a
//! kind:30382 signed by anyone else is ignored.

use std::collections::HashMap;

use nostr::Event;
use serde_json::{json, Value};

use crate::client::BuzzClient;
use crate::commands::agents::normalize_relay_self_hex;
use crate::error::CliError;

/// Event kind of the relay-signed verified profile projection.
const KIND: u16 = buzz_core::kind::KIND_VERIFIED_PROFILE as u16;

/// Subjects per query, keeping each REQ inside relay filter limits.
const SUBJECTS_PER_QUERY: usize = 128;

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

/// Where a community publishes verified profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Authority {
    /// The relay key that signs them.
    pub pubkey: String,
    /// `public_profile.searchable`: `POST /profiles/search` matches verified names.
    pub searchable: bool,
}

/// Read the relay key that signs verified profiles from a NIP-11 document.
///
/// `Ok(None)`: the community does not advertise verified profiles, so every
/// kind:30382 is ignored. A malformed advertisement is an error rather than
/// "no verified profiles", so an agent never reads a failed check as an
/// authoritative absence.
pub(crate) fn parse_discovery(info: &Value) -> Result<Option<Authority>, CliError> {
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
    if profile.get("kind").and_then(Value::as_u64) != Some(u64::from(KIND)) {
        return Err(malformed("malformed public_profile discovery"));
    }
    Ok(Some(Authority {
        pubkey: normalize_relay_self_hex(self_hex)?,
        searchable: profile.get("searchable") == Some(&Value::Bool(true)),
    }))
}

/// The key that signs this community's verified profiles, if it advertises any.
pub(crate) async fn discover_authority(client: &BuzzClient) -> Result<Option<Authority>, CliError> {
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
/// Anything not signed by `authority`, not of kind 30382, or with a bad id or
/// signature is ignored. Per subject the newest event is the head, as for any
/// addressable event (the lower id breaks a same-second tie); a head that is not a valid active projection (including
/// a retraction) leaves the subject with no verified profile, so a stale older
/// name never resurfaces.
pub(crate) fn verified_profiles_from_events(
    events: &[Value],
    authority: &str,
) -> HashMap<String, VerifiedProfile> {
    let mut heads: HashMap<String, Event> = HashMap::new();
    for raw in events {
        let Ok(event) = serde_json::from_value::<Event>(raw.clone()) else {
            continue;
        };
        let Some(subject) = signed_subject(&event, authority) else {
            continue;
        };
        // Newest wins; the lower id breaks a same-second tie.
        if heads.get(&subject).is_none_or(|current| {
            event.created_at > current.created_at
                || (event.created_at == current.created_at && event.id < current.id)
        }) {
            heads.insert(subject, event);
        }
    }
    heads
        .into_iter()
        .filter_map(|(subject, event)| parse_projection(&event).map(|p| (subject, p)))
        .collect()
}

/// The subject a kind:30382 event addresses (its first `d` tag, a lowercase hex
/// pubkey), when the event is validly signed by `authority`.
fn signed_subject(event: &Event, authority: &str) -> Option<String> {
    if event.kind.as_u16() != KIND || event.pubkey.to_hex() != authority || event.verify().is_err()
    {
        return None;
    }
    let subject = tag_value(event, "d")?;
    (subject.len() == 64
        && subject
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then(|| subject.to_owned())
}

fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().map(String::as_str) == Some(name))
        .and_then(|tag| tag.get(1))
        .map(String::as_str)
}

/// Tags a projection carries, each exactly once; `full_name` is optional.
const PROJECTION_TAGS: [&str; 7] = [
    "d",
    "p",
    "active",
    "display_name",
    "username",
    "issuer_label",
    "verified_at",
];

/// Read the NIP-FI projection with the spec's structural checks: empty
/// content; exactly the projection tags, each once with one value; `p` equal
/// to `d`; `active` `true`; a decimal `verified_at` no later than
/// `created_at`; and names that pass [`valid_name`]. Anything else is not a
/// verified profile.
fn parse_projection(event: &Event) -> Option<VerifiedProfile> {
    if !event.content.is_empty() {
        return None;
    }
    let mut tags: HashMap<&str, &str> = HashMap::new();
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        let [name, value] = tag else {
            return None;
        };
        let known = PROJECTION_TAGS.contains(&name.as_str()) || name == "full_name";
        if !known || tags.insert(name.as_str(), value.as_str()).is_some() {
            return None;
        }
    }
    let get = |name| tags.get(name).copied();
    if !PROJECTION_TAGS.iter().all(|name| tags.contains_key(name))
        || get("p") != get("d")
        || get("active") != Some("true")
    {
        return None;
    }
    let name = |name| get(name).filter(|v| valid_name(v));
    let full_name = match get("full_name") {
        Some(full_name) => Some(valid_name(full_name).then_some(full_name)?),
        None => None,
    };
    let verified_at = get("verified_at")?;
    if verified_at.is_empty() || !verified_at.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let verified_at: u64 = verified_at.parse().ok()?;
    if verified_at > event.created_at.as_secs() {
        return None;
    }
    Some(VerifiedProfile {
        display_name: name("display_name")?.to_owned(),
        username: get("username").filter(|u| valid_username(u))?.to_owned(),
        full_name: full_name.map(str::to_owned),
        issuer_label: name("issuer_label")?.to_owned(),
        verified_at,
    })
}

/// Display text: 1 to 64 characters, no surrounding whitespace, no control
/// characters (Cc).
///
/// Looser than the spec, the relay and the mobile client, which also require
/// NFC and reject the Cf, Cs, Co, Cn, Zl and Zp categories. The relay applies
/// those rules before it signs, so a name that passes here but fails there
/// would need a relay-signed event that breaks its own rules.
fn valid_name(value: &str) -> bool {
    (1..=64).contains(&value.chars().count())
        && value.trim() == value
        && !value.chars().any(char::is_control)
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
        let found = |profile| parse_discovery(&advertised(profile)).unwrap().unwrap();
        assert_eq!(found(json!({"kind": 30382})).pubkey, key);
        // `searchable` must be `true` for profile search to match verified names.
        assert!(!found(json!({"kind": 30382})).searchable);
        assert!(!found(json!({"kind": 30382, "searchable": "yes"})).searchable);
        assert!(found(json!({"kind": 30382, "searchable": true})).searchable);
        // Not advertised: absent or without `public_profile`.
        assert_eq!(parse_discovery(&json!({"self": key})).unwrap(), None);
        assert_eq!(
            parse_discovery(&json!({"self": key, "federated_identity": {}})).unwrap(),
            None
        );
        // Advertised but malformed is a failed check, not an absence.
        for bad in [
            advertised(json!(null)),
            advertised(json!({})),
            advertised(json!({"kind": 1})),
            json!({"federated_identity": {"public_profile": {"kind": 30382}}}),
            json!({"self": "nothex", "federated_identity": {"public_profile": {"kind": 30382}}}),
            json!({"self": key, "federated_identity": "on"}),
        ] {
            assert!(parse_discovery(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_relay_signed_active_projection_is_verified() {
        let relay = Keys::generate();
        let authority = relay.public_key().to_hex();
        let subject = Keys::generate().public_key().to_hex();
        let tags = tag_pairs(&subject, &[("full_name", "Alice Example")]);
        let event = sign(&relay, 30382, "", &tags, AT);
        let found = verified_profiles_from_events(&[event], &authority);
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
        // `full_name` is optional and omitted when absent.
        let event = sign(&relay, 30382, "", &tag_pairs(&subject, &[]), AT);
        let found = verified_profiles_from_events(&[event], &authority);
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
    fn unusable_projections_are_ignored() {
        let relay = Keys::generate();
        let authority = relay.public_key().to_hex();
        let subject = Keys::generate().public_key().to_hex();
        let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
            ("retracted", vec![("active", "false")]),
            ("bad username", vec![("username", "Alice")]),
            ("empty username", vec![("username", "")]),
            ("empty display_name", vec![("display_name", "")]),
            ("non-numeric verified_at", vec![("verified_at", "soon")]),
            ("signed verified_at", vec![("verified_at", "+1")]),
            (
                "verified_at after created_at",
                vec![("verified_at", "1800000001")],
            ),
            ("p is not d", vec![("p", "00")]),
            ("padded display_name", vec![("display_name", " alice")]),
            (
                "control char in issuer_label",
                vec![("issuer_label", "Ex\u{7}ample")],
            ),
        ];
        for (label, overrides) in cases {
            let mut tags = tag_pairs(&subject, &[]);
            for (key, value) in &overrides {
                tags.iter_mut().find(|(k, _)| k == key).unwrap().1 = value;
            }
            let event = sign(&relay, 30382, "", &tags, AT);
            assert!(
                verified_profiles_from_events(&[event], &authority).is_empty(),
                "{label}"
            );
        }
        let missing: Vec<_> = tag_pairs(&subject, &[])
            .into_iter()
            .filter(|(k, _)| *k != "issuer_label")
            .collect();
        let event = sign(&relay, 30382, "", &missing, AT);
        assert!(verified_profiles_from_events(&[event], &authority).is_empty());

        // Content, an unknown or repeated tag, or a tag with more than one value.
        let content = sign(&relay, 30382, "hi", &tag_pairs(&subject, &[]), AT);
        let extra = sign(&relay, 30382, "", &tag_pairs(&subject, &[("x", "1")]), AT);
        let repeated = sign(
            &relay,
            30382,
            "",
            &tag_pairs(&subject, &[("username", "bob")]),
            AT,
        );
        let mut tags: Vec<Tag> = tag_pairs(&subject, &[])
            .iter()
            .map(|(k, v)| Tag::parse([*k, *v]).unwrap())
            .collect();
        tags.push(Tag::parse(["full_name", "Alice", "extra"]).unwrap());
        let long_tag = EventBuilder::new(Kind::Custom(30382), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(AT))
            .sign_with_keys(&relay)
            .unwrap();
        let long_tag = serde_json::to_value(long_tag).unwrap();
        for (label, event) in [
            ("content", content),
            ("extra", extra),
            ("repeated", repeated),
            ("long tag", long_tag),
        ] {
            assert!(
                verified_profiles_from_events(&[event], &authority).is_empty(),
                "{label}"
            );
        }
    }

    #[test]
    fn the_lower_id_wins_a_same_second_tie() {
        let relay = Keys::generate();
        let authority = relay.public_key().to_hex();
        let subject = Keys::generate().public_key().to_hex();
        let mut events: Vec<Value> = ["alice", "alice2"]
            .iter()
            .map(|username| {
                let mut tags = tag_pairs(&subject, &[]);
                tags[4] = ("username", username);
                sign(&relay, 30382, "", &tags, AT)
            })
            .collect();
        events.sort_by_key(|e| e["id"].as_str().unwrap().to_owned());
        let lower = events[0]["tags"][4][1].as_str().unwrap().to_owned();
        for order in [events.clone(), events.into_iter().rev().collect()] {
            let found = verified_profiles_from_events(&order, &authority);
            assert_eq!(found[&subject].username, lower);
        }
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
