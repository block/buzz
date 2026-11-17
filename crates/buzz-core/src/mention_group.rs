//! NIP-MG wire validation, independent of transport admission and persistence.
//!
//! These parsers do not verify signatures, actor roles, freshness, the relay's
//! configured message-size limit, or version preconditions against stored state.
//! Those checks belong to the relay and its admitted mutation transaction.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use nostr::{Event, PublicKey};
use uuid::Uuid;

use crate::kind::{
    event_kind_u32, RELAY_ADMIN_DELETE_MENTION_GROUP, RELAY_ADMIN_PUT_MENTION_GROUP,
    RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
};

/// Reserved addressable-state coordinate for community management settings.
pub const SETTINGS_D_TAG: &str = "settings";
/// Maximum number of Unicode scalar values in a normalized group name.
pub const MAX_NAME_CHARS: usize = 120;
/// Maximum distinct members in a group replacement. Keep in sync with
/// `buzzPushHellthreadParticipantLimit` in
/// `mobile/lib/shared/push/push_contract.dart` so one group fits the mobile
/// push recipient gate. Clients discover this value through NIP-11.
pub const MAX_MEMBERS: usize = 20;
/// Maximum distinct default channels in a group replacement.
pub const MAX_DEFAULT_CHANNELS: usize = 32;
const MAX_CLIENT_ID_UTF16: usize = 128;
const MAX_AUTH_CONDITIONS_BYTES: usize = 4096;

/// Validated complete replacement or deletion request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// Canonical, non-nil community-local group identity.
    pub id: Uuid,
    /// Expected current version; zero requests creation at an unused identity.
    pub prev: i64,
    /// Requested transition. Empty membership remains a live put.
    pub operation: Operation,
}

/// Group mutation carried by a command kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Replace the entire name and explicit member set.
    Put {
        /// Trimmed display name, preserving case, spelling and internal spaces.
        name: String,
        /// Lowercase hex public keys, sorted and deduplicated, including self.
        members: Vec<String>,
        /// Canonical default-channel UUIDs, sorted and deduplicated.
        channels: Vec<Uuid>,
    },
    /// Retire this identity permanently; the store publishes a tombstone.
    Delete,
}

/// Community mention-group management setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manage {
    /// Management is restricted according to the community's admin policy.
    Admins,
    /// Management is available according to the community's everyone policy.
    Everyone,
}

impl Manage {
    /// Stable value used in command and settings-state tags.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admins => "admins",
            Self::Everyone => "everyone",
        }
    }
}

impl fmt::Display for Manage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Manage {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "admins" => Ok(Self::Admins),
            "everyone" => Ok(Self::Everyone),
            _ => Err("manage must be admins or everyone".into()),
        }
    }
}

/// Validated request to replace the community management setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyCommand {
    /// Expected settings revision; zero identifies the implicit initial state.
    pub prev: i64,
    /// Requested stable policy value; authorization is a relay concern.
    pub manage: Manage,
}

/// Parse a put/delete command and normalize its display name and member set.
///
/// The parser accepts only this operation's semantic tags and bounded existing
/// `auth`, `client-id`, and `ms` transport annotations. It never infers a member
/// from the signer: self-membership must be explicitly present as a `p` tag.
pub fn parse_command(event: &Event) -> Result<Command, String> {
    let put = match event_kind_u32(event) {
        RELAY_ADMIN_PUT_MENTION_GROUP => true,
        RELAY_ADMIN_DELETE_MENTION_GROUP => false,
        _ => return Err("not a mention group put/delete command".into()),
    };
    empty_content(event)?;
    let mut id = None;
    let mut prev = None;
    let mut name = None;
    let mut members = BTreeSet::new();
    let mut channels = BTreeSet::new();
    let mut annotations = Annotations::default();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts.first().map(String::as_str) {
            Some("d") => singleton(&mut id, parts)?,
            Some("prev") => singleton(&mut prev, parts)?,
            Some("name") if put => singleton(&mut name, parts)?,
            Some("p") if put => {
                let value = pair(parts)?;
                let key = PublicKey::from_hex(value)
                    .map_err(|_| "member must be a 64-character hex public key")?;
                members.insert(key.to_hex());
                if members.len() > MAX_MEMBERS {
                    return Err(format!("a group can have at most {MAX_MEMBERS} members"));
                }
            }
            Some("channel") if put => {
                let id = crate::artifact::canonical_uuid(pair(parts)?).map_err(str::to_owned)?;
                channels.insert(id);
                if channels.len() > MAX_DEFAULT_CHANNELS {
                    return Err("at most 32 distinct default channels are allowed".into());
                }
            }
            _ => annotations.accept(parts)?,
        }
    }
    let id = crate::artifact::canonical_uuid(required(id, "d")?).map_err(str::to_owned)?;
    let prev = version(required(prev, "prev")?)?;
    let operation = if put {
        let name = required(name, "name")?.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_CHARS
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        {
            return Err(
                "name must be 1..120 Unicode characters of single-line text without controls"
                    .into(),
            );
        }
        Operation::Put {
            name: name.into(),
            members: members.into_iter().collect(),
            channels: channels.into_iter().collect(),
        }
    } else {
        Operation::Delete
    };
    Ok(Command {
        id,
        prev,
        operation,
    })
}

/// Parse a settings command containing exactly one `manage` and one `prev` tag.
///
/// The reserved `d=settings` coordinate belongs to the relay-signed projection;
/// commands contain no `d` tag. Actor authorization is deliberately separate.
pub fn parse_policy_command(event: &Event) -> Result<PolicyCommand, String> {
    if event_kind_u32(event) != RELAY_ADMIN_SET_MENTION_GROUP_POLICY {
        return Err("not a mention group policy command".into());
    }
    empty_content(event)?;
    let mut prev = None;
    let mut manage = None;
    let mut annotations = Annotations::default();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts.first().map(String::as_str) {
            Some("prev") => singleton(&mut prev, parts)?,
            Some("manage") => singleton(&mut manage, parts)?,
            _ => annotations.accept(parts)?,
        }
    }
    Ok(PolicyCommand {
        prev: version(required(prev, "prev")?)?,
        manage: required(manage, "manage")?.parse()?,
    })
}

fn empty_content(event: &Event) -> Result<(), String> {
    if !event.content.is_empty() {
        return Err("mention group commands require empty content".into());
    }
    Ok(())
}

fn pair(parts: &[String]) -> Result<&str, String> {
    if parts.len() != 2 {
        return Err("command fields require exactly two tag elements".into());
    }
    Ok(&parts[1])
}

fn singleton<'a>(field: &mut Option<&'a str>, parts: &'a [String]) -> Result<(), String> {
    if field.replace(pair(parts)?).is_some() {
        return Err("duplicate command field".into());
    }
    Ok(())
}

fn required<'a>(field: Option<&'a str>, label: &str) -> Result<&'a str, String> {
    field.ok_or_else(|| format!("missing {label} tag"))
}

fn version(value: &str) -> Result<i64, String> {
    let parsed = value
        .parse::<i64>()
        .map_err(|_| "prev must be a canonical nonnegative signed-64-bit decimal")?;
    if parsed < 0 || parsed.to_string() != value {
        return Err("prev must be a canonical nonnegative signed-64-bit decimal".into());
    }
    Ok(parsed)
}

#[derive(Default)]
struct Annotations {
    auth: bool,
    client_id: bool,
    ms: bool,
}

impl Annotations {
    fn accept(&mut self, parts: &[String]) -> Result<(), String> {
        let seen = match parts.first().map(String::as_str) {
            Some("auth") => {
                // NIP-OA verification and condition semantics stay with the
                // authenticated transport. Only bound the envelope here.
                if parts.len() != 4
                    || parts[1].len() != 64
                    || !parts[1].bytes().all(|b| b.is_ascii_hexdigit())
                    || parts[2].len() > MAX_AUTH_CONDITIONS_BYTES
                    || parts[3].len() != 128
                    || !parts[3].bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("invalid auth annotation".into());
                }
                &mut self.auth
            }
            Some("client-id") => {
                let value = pair(parts)?;
                if value.is_empty()
                    || value.encode_utf16().count() > MAX_CLIENT_ID_UTF16
                    || value.chars().any(char::is_control)
                {
                    return Err("invalid client-id annotation".into());
                }
                &mut self.client_id
            }
            Some("ms") => {
                let value = pair(parts)?;
                let parsed = version(value).map_err(|_| "invalid ms annotation")?;
                if parsed > 999 {
                    return Err("invalid ms annotation".into());
                }
                &mut self.ms
            }
            _ => return Err("unexpected command tag".into()),
        };
        if std::mem::replace(seen, true) {
            return Err("duplicate transport annotation".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const ID: &str = "0659678f-fddb-40df-a285-35f6b41db019";

    fn fields(values: &[&[&str]]) -> Vec<Vec<String>> {
        values
            .iter()
            .map(|v| v.iter().map(|s| (*s).into()).collect())
            .collect()
    }

    fn put_tags() -> Vec<Vec<String>> {
        fields(&[&["d", ID], &["prev", "0"], &["name", "Support team"]])
    }

    fn signed(keys: &Keys, kind: u32, tags: Vec<Vec<String>>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .allow_self_tagging()
            .tags(tags.into_iter().map(|parts| Tag::parse(parts).unwrap()))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn event(kind: u32, tags: Vec<Vec<String>>) -> Event {
        signed(&Keys::generate(), kind, tags, "")
    }

    #[test]
    fn put_normalizes_the_explicit_set_without_dropping_the_signer() {
        let keys = Keys::generate();
        let self_key = keys.public_key().to_hex();
        let other = Keys::generate().public_key().to_hex();
        let mut tags = put_tags();
        tags[2][1] = "  SRE — 夜間  ".into();
        tags.extend(fields(&[
            &["p", &other.to_uppercase()],
            &["p", &self_key],
            &["p", &other],
        ]));
        let command = signed(&keys, RELAY_ADMIN_PUT_MENTION_GROUP, tags, "");
        let mut expected = vec![self_key, other];
        expected.sort();
        assert_eq!(
            parse_command(&command).unwrap(),
            Command {
                id: Uuid::parse_str(ID).unwrap(),
                prev: 0,
                operation: Operation::Put {
                    name: "SRE — 夜間".into(),
                    members: expected,
                    channels: vec![]
                },
            }
        );
    }

    #[test]
    fn empty_put_update_and_delete_have_distinct_public_operations() {
        let empty = event(RELAY_ADMIN_PUT_MENTION_GROUP, put_tags());
        assert!(matches!(parse_command(&empty).unwrap().operation,
            Operation::Put { members, .. } if members.is_empty()));
        let mut update = put_tags();
        update[1][1] = i64::MAX.to_string();
        assert_eq!(
            parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, update))
                .unwrap()
                .prev,
            i64::MAX
        );
        let delete = event(
            RELAY_ADMIN_DELETE_MENTION_GROUP,
            fields(&[&["d", ID], &["prev", "3"]]),
        );
        assert_eq!(parse_command(&delete).unwrap().operation, Operation::Delete);
        for extra in [
            vec!["name".into(), "Hidden".into()],
            vec!["p".into(), "ab".repeat(32)],
        ] {
            let mut tags = fields(&[&["d", ID], &["prev", "3"]]);
            tags.push(extra);
            assert!(parse_command(&event(RELAY_ADMIN_DELETE_MENTION_GROUP, tags)).is_err());
        }
    }

    #[test]
    fn ambiguous_or_malformed_command_fields_are_rejected() {
        for field in 0..3 {
            let mut duplicate = put_tags();
            duplicate.push(duplicate[field].clone());
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, duplicate)).is_err());
            let mut missing = put_tags();
            missing.remove(field);
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, missing)).is_err());
            let mut extra_value = put_tags();
            extra_value[field].push("extra".into());
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, extra_value)).is_err());
        }
        for id in [
            ID.to_uppercase(),
            ID.replace('-', ""),
            Uuid::nil().to_string(),
            "settings".into(),
        ] {
            let mut tags = put_tags();
            tags[0][1] = id;
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_err());
        }
        for prev in [
            "",
            "-1",
            "+1",
            "01",
            " 1",
            "1 ",
            "1.0",
            "9223372036854775808",
        ] {
            let mut tags = put_tags();
            tags[1][1] = prev.into();
            assert!(
                parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_err(),
                "{prev}"
            );
        }
        for extra in fields(&[
            &["h", ID],
            &["version", "1"],
            &["deleted", "true"],
            &["name"],
            &["p", "npub1bad"],
            &["p", "ab", "relay"],
        ]) {
            let mut tags = put_tags();
            tags.push(extra);
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_err());
        }
        assert!(parse_command(&signed(
            &Keys::generate(),
            RELAY_ADMIN_PUT_MENTION_GROUP,
            put_tags(),
            "hidden"
        ))
        .is_err());
        assert!(parse_command(&event(crate::kind::KIND_MENTION_GROUP_STATE, put_tags())).is_err());
    }

    #[test]
    fn names_use_unicode_scalar_limits_and_groups_bound_distinct_members() {
        for (name, accepted) in [
            ("夜".repeat(120), true),
            ("夜".repeat(121), false),
            ("   ".into(), false),
            ("a\nb".into(), false),
            ("a\tb".into(), false),
            ("a\u{7f}b".into(), false),
            ("a\u{2028}b".into(), false),
            ("a\u{2029}b".into(), false),
        ] {
            let mut tags = put_tags();
            tags[2][1] = name;
            assert_eq!(
                parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_ok(),
                accepted
            );
        }
        let mut tags = put_tags();
        for _ in 0..20 {
            tags.push(vec!["p".into(), Keys::generate().public_key().to_hex()]);
        }
        tags.push(vec!["p".into(), tags[3][1].to_uppercase()]);
        assert!(
            matches!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags.clone())).unwrap().operation,
            Operation::Put { members, .. } if members.len() == 20)
        );
        tags.push(vec!["p".into(), Keys::generate().public_key().to_hex()]);
        assert_eq!(
            parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).unwrap_err(),
            "a group can have at most 20 members"
        );
    }

    #[test]
    fn group_member_limit_matches_mobile_push_gate() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../mobile/lib/shared/push/push_contract.dart"
        ))
        .expect("read the mobile push contract");
        let initializer = source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("const "))
            .filter_map(|declaration| declaration.split_once('='))
            .find_map(|(name, value)| {
                (name.split_whitespace().last() == Some("buzzPushHellthreadParticipantLimit"))
                    .then_some(value)
            })
            .expect("mobile push participant limit must be declared");
        let mobile_limit: usize = initializer
            .split_once(';')
            .expect("mobile push participant limit must end with a semicolon")
            .0
            .trim()
            .parse()
            .expect("mobile push participant limit must be an integer");
        assert_eq!(
            MAX_MEMBERS, mobile_limit,
            "group members and the mobile push recipient gate must have the same limit"
        );
    }

    #[test]
    fn default_channels_are_a_bounded_canonical_set() {
        let mut tags = put_tags();
        for value in (1..=32).rev().chain([1, 32]) {
            tags.push(vec!["channel".into(), Uuid::from_u128(value).to_string()]);
        }
        let parsed = parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags.clone())).unwrap();
        assert!(matches!(parsed.operation, Operation::Put { channels, .. }
            if channels == (1..=32).map(Uuid::from_u128).collect::<Vec<_>>()));
        tags.push(vec!["channel".into(), Uuid::from_u128(33).to_string()]);
        assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_err());
        for channel in [
            ID.to_uppercase(),
            Uuid::nil().to_string(),
            "private-room".into(),
        ] {
            let mut tags = put_tags();
            tags.push(vec!["channel".into(), channel]);
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_err());
        }
        let mut tags = fields(&[&["d", ID], &["prev", "1"]]);
        tags.push(vec!["channel".into(), ID.into()]);
        assert!(parse_command(&event(RELAY_ADMIN_DELETE_MENTION_GROUP, tags)).is_err());
    }

    #[test]
    fn only_bounded_singleton_transport_annotations_are_accepted() {
        let auth = vec!["auth".into(), "ab".repeat(32), "".into(), "cd".repeat(64)];
        let annotations = vec![
            auth,
            vec!["client-id".into(), "x".repeat(128)],
            vec!["ms".into(), "999".into()],
        ];
        let mut tags = put_tags();
        tags.extend(annotations.clone());
        assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_ok());
        for annotation in annotations {
            let mut tags = put_tags();
            tags.extend([annotation.clone(), annotation]);
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_err());
        }
        let mut invalid = fields(&[
            &["client-id", ""],
            &["client-id", "a", "extra"],
            &["ms", "1000"],
            &["ms", "00"],
            &["ms", "-1"],
            &["auth", "ab"],
            &["unknown", "value"],
            &["-"],
        ]);
        invalid.push(vec!["client-id".into(), "😀".repeat(65)]);
        invalid.push(vec![
            "auth".into(),
            "ab".repeat(32),
            "x".repeat(4097),
            "cd".repeat(64),
        ]);
        for annotation in invalid {
            let mut tags = put_tags();
            tags.push(annotation);
            assert!(parse_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, tags)).is_err());
        }
    }

    #[test]
    fn policy_wire_values_are_exact_and_do_not_accept_group_fields() {
        for (wire, expected) in [("admins", Manage::Admins), ("everyone", Manage::Everyone)] {
            let policy = event(
                RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
                fields(&[&["manage", wire], &["prev", "0"]]),
            );
            assert_eq!(
                parse_policy_command(&policy).unwrap(),
                PolicyCommand {
                    prev: 0,
                    manage: expected
                }
            );
            assert_eq!(expected.to_string(), wire);
        }
        for wire in ["Admins", " everyone", "", "members"] {
            let policy = event(
                RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
                fields(&[&["manage", wire], &["prev", "0"]]),
            );
            assert!(parse_policy_command(&policy).is_err());
        }
        for extra in fields(&[
            &["manage", "admins"],
            &["prev", "0"],
            &["d", SETTINGS_D_TAG],
            &["name", "Team"],
            &["p", "ab"],
        ]) {
            let mut tags = fields(&[&["manage", "admins"], &["prev", "0"]]);
            tags.push(extra);
            assert!(
                parse_policy_command(&event(RELAY_ADMIN_SET_MENTION_GROUP_POLICY, tags)).is_err()
            );
        }
        for tags in [
            fields(&[&["prev", "0"]]),
            fields(&[&["manage", "admins"]]),
            fields(&[&["manage", "admins"], &["prev", "01"]]),
        ] {
            assert!(
                parse_policy_command(&event(RELAY_ADMIN_SET_MENTION_GROUP_POLICY, tags)).is_err()
            );
        }
        assert!(parse_policy_command(&signed(
            &Keys::generate(),
            RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
            fields(&[&["manage", "admins"], &["prev", "0"]]),
            "hidden"
        ))
        .is_err());
        assert!(parse_policy_command(&event(RELAY_ADMIN_PUT_MENTION_GROUP, put_tags())).is_err());
    }
}
