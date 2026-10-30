//! NIP-UG user groups: named, mentionable sets of community members.
//!
//! A group is one addressable event ([`KIND_USER_GROUP`]) with the handle in its
//! `d` tag and the members in `p` tags. This module is the single parser and
//! write-authorization policy for relay, SDK, and CLI. See `docs/nips/NIP-UG.md`.

use std::collections::{BTreeMap, HashSet};

use nostr::{Event, Tag};
use thiserror::Error;

use crate::kind::{event_kind_u32, KIND_USER_GROUP};

/// Maximum bytes in a handle.
pub const MAX_HANDLE_LEN: usize = 64;
/// Maximum bytes in a display name.
pub const MAX_NAME_LEN: usize = 128;
/// Maximum bytes in a description (the event `content`).
pub const MAX_DESCRIPTION_LEN: usize = 1024;
/// Maximum members in one group.
pub const MAX_MEMBERS: usize = 500;
/// Maximum group admins in one group.
pub const MAX_ADMINS: usize = 20;
/// Maximum distinct groups one message may mention.
pub const MAX_GROUP_MENTIONS_PER_EVENT: usize = 5;
/// Handles that Slack and Buzz clients treat as broadcast mentions.
pub const RESERVED_HANDLES: [&str; 4] = ["channel", "here", "everyone", "all"];
/// Prefix of the fourth `p` tag element on a group mention: `group:<handle>`.
pub const GROUP_MENTION_MARKER_PREFIX: &str = "group:";

/// Why an event is not a valid user group, or why a write is refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UserGroupError {
    /// The handle is empty, too long, reserved, or has a forbidden character.
    #[error("invalid handle: {0}")]
    Handle(String),
    /// The event is not a valid user group.
    #[error("{0}")]
    Invalid(String),
    /// The writer may not make this change.
    #[error("{0}")]
    Forbidden(String),
}

/// Who may edit a group beyond its creator, group admins, and community admins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditPolicy {
    /// Only the creator, group admins, and community admins.
    #[default]
    Admins,
    /// Any community member may change name, description, members, and status.
    Open,
}

impl EditPolicy {
    /// The wire value of the `policy` tag.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admins => "admins",
            Self::Open => "open",
        }
    }
}

/// A parsed, valid user group head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserGroup {
    /// Normalized handle (the `d` tag), without a leading `@`.
    pub handle: String,
    /// Display name.
    pub name: String,
    /// Description (the event `content`).
    pub description: String,
    /// Lowercase hex pubkey of the original creator. Never changes.
    pub creator: String,
    /// Lowercase hex pubkeys of group admins.
    pub admins: Vec<String>,
    /// Lowercase hex pubkeys of members, in event order.
    pub members: Vec<String>,
    /// Edit policy.
    pub policy: EditPolicy,
    /// A disabled group keeps its handle but notifies no one.
    pub disabled: bool,
}

/// The writer of a group event, as the relay resolved them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    /// Lowercase hex pubkey that signed the event.
    pub pubkey: String,
    /// The signer is a community owner or admin.
    pub community_admin: bool,
}

/// Normalize a handle: trim, drop one leading `@`, lowercase, then validate.
///
/// A handle is 1 to 64 bytes of ASCII letters, digits, `.`, `-`, and `_`. It
/// starts with a letter or digit and is not one of [`RESERVED_HANDLES`].
pub fn normalize_handle(raw: &str) -> Result<String, UserGroupError> {
    let trimmed = raw.trim();
    let handle = trimmed.strip_prefix('@').unwrap_or(trimmed);
    let reject = |why: &str| Err(UserGroupError::Handle(format!("{why} (got {raw:?})")));
    if handle.is_empty() {
        return reject("handle must not be empty");
    }
    if handle.len() > MAX_HANDLE_LEN {
        return reject(&format!("handle must be at most {MAX_HANDLE_LEN} bytes"));
    }
    if !handle
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return reject("handle may only contain letters, digits, '.', '-', and '_'");
    }
    if !handle.as_bytes()[0].is_ascii_alphanumeric() {
        return reject("handle must start with a letter or digit");
    }
    let lower = handle.to_ascii_lowercase();
    if RESERVED_HANDLES.contains(&lower.as_str()) {
        return reject("handle is reserved for broadcast mentions");
    }
    Ok(lower)
}

/// Normalize a pubkey tag value to lowercase hex, or reject it.
pub fn normalize_pubkey(raw: &str) -> Result<String, UserGroupError> {
    if raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(raw.to_ascii_lowercase())
    } else {
        Err(UserGroupError::Invalid(format!(
            "pubkey must be 64 hex characters (got {raw:?})"
        )))
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, UserGroupError> {
    Err(UserGroupError::Invalid(message.into()))
}

fn single<'a>(
    tags: &[&'a [String]],
    name: &str,
    required: bool,
) -> Result<Option<&'a str>, UserGroupError> {
    let mut found = tags.iter().filter(|t| t[0] == name);
    let first = found.next();
    if found.next().is_some() {
        return invalid(format!("group must have at most one {name} tag"));
    }
    match first {
        Some(tag) => match tag.get(1) {
            Some(value) => Ok(Some(value.as_str())),
            None => invalid(format!("{name} tag needs a value")),
        },
        None if required => invalid(format!("group must have exactly one {name} tag")),
        None => Ok(None),
    }
}

fn distinct_pubkeys(
    tags: &[&[String]],
    name: &str,
    max: usize,
) -> Result<Vec<String>, UserGroupError> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tag in tags.iter().filter(|t| t[0] == name) {
        let Some(value) = tag.get(1) else {
            return invalid(format!("{name} tag needs a pubkey"));
        };
        if name == "p" && tag.len() > 2 && tag.iter().skip(2).any(|v| !v.is_empty()) {
            return invalid("member p tags must not carry a marker");
        }
        let pubkey = normalize_pubkey(value)?;
        if !seen.insert(pubkey.clone()) {
            return invalid(format!("duplicate {name} pubkey {pubkey}"));
        }
        out.push(pubkey);
    }
    if out.len() > max {
        return invalid(format!("group has more than {max} {name} tags"));
    }
    Ok(out)
}

/// Parse and validate a user group event.
///
/// Checks the kind, exactly one `d`, `name`, and `creator` tag, no `h` tag,
/// distinct 64-hex `p` and `admin` pubkeys within [`MAX_MEMBERS`] and
/// [`MAX_ADMINS`], the optional `policy` and `status` tags, and the length
/// limits. It does not check signatures, authorship, or relay state.
pub fn parse_user_group(event: &Event) -> Result<UserGroup, UserGroupError> {
    if event_kind_u32(event) != KIND_USER_GROUP {
        return invalid(format!("expected kind {KIND_USER_GROUP}"));
    }
    let raw: Vec<&[String]> = event
        .tags
        .iter()
        .map(Tag::as_slice)
        .filter(|t| !t.is_empty())
        .collect();
    parse_parts(&event.content, &raw)
}

fn parse_parts(content: &str, raw: &[&[String]]) -> Result<UserGroup, UserGroupError> {
    if raw.iter().any(|t| t[0] == "h") {
        return invalid("user group must not carry an h tag");
    }
    let d = single(raw, "d", true)?.unwrap_or_default();
    let handle = normalize_handle(d)?;
    if d != handle {
        return invalid("d tag must be the normalized handle");
    }
    let name = single(raw, "name", true)?.unwrap_or_default().to_string();
    if name.trim().is_empty() || name.trim() != name {
        return invalid("name must not be empty or padded with spaces");
    }
    if name.len() > MAX_NAME_LEN || name.chars().any(char::is_control) {
        return invalid(format!(
            "name must be at most {MAX_NAME_LEN} bytes with no control characters"
        ));
    }
    if content.len() > MAX_DESCRIPTION_LEN {
        return invalid(format!(
            "description exceeds {MAX_DESCRIPTION_LEN} bytes (got {})",
            content.len()
        ));
    }
    let creator = normalize_pubkey(single(raw, "creator", true)?.unwrap_or_default())?;
    let policy = match single(raw, "policy", false)? {
        None | Some("admins") => EditPolicy::Admins,
        Some("open") => EditPolicy::Open,
        Some(other) => return invalid(format!("policy must be admins or open (got {other:?})")),
    };
    let disabled = match single(raw, "status", false)? {
        None | Some("active") => false,
        Some("disabled") => true,
        Some(other) => {
            return invalid(format!("status must be active or disabled (got {other:?})"))
        }
    };
    Ok(UserGroup {
        handle,
        name,
        description: content.to_string(),
        creator,
        admins: distinct_pubkeys(raw, "admin", MAX_ADMINS)?,
        members: distinct_pubkeys(raw, "p", MAX_MEMBERS)?,
        policy,
        disabled,
    })
}

impl UserGroup {
    /// The event tags for this group, in canonical order.
    pub fn tags(&self) -> Vec<Vec<String>> {
        let mut tags = vec![
            vec!["d".into(), self.handle.clone()],
            vec!["name".into(), self.name.clone()],
            vec!["creator".into(), self.creator.clone()],
        ];
        tags.extend(self.admins.iter().map(|a| vec!["admin".into(), a.clone()]));
        tags.extend(self.members.iter().map(|m| vec!["p".into(), m.clone()]));
        if self.policy != EditPolicy::Admins {
            tags.push(vec!["policy".into(), self.policy.as_str().into()]);
        }
        if self.disabled {
            tags.push(vec!["status".into(), "disabled".into()]);
        }
        tags
    }

    /// Check every field against the same rules [`parse_user_group`] applies to
    /// a received event.
    pub fn validate(&self) -> Result<(), UserGroupError> {
        let tags = self.tags();
        let raw: Vec<&[String]> = tags.iter().map(Vec::as_slice).collect();
        parse_parts(&self.description, &raw).map(|_| ())
    }
}

/// The newest valid head among events that share one handle.
///
/// Heads from different authors can coexist briefly; the newest `created_at`
/// wins and the lowest event id breaks ties (the NIP-01 rule). Events that fail
/// [`parse_user_group`] or carry another handle are ignored.
pub fn resolve_head<'a>(handle: &str, events: &'a [Event]) -> Option<(&'a Event, UserGroup)> {
    events
        .iter()
        .filter_map(|e| parse_user_group(e).ok().map(|g| (e, g)))
        .filter(|(_, g)| g.handle == handle)
        .min_by(|(a, _), (b, _)| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.to_hex().cmp(&b.id.to_hex()))
        })
}

/// Decide whether `writer` may publish `incoming` over `prior`.
///
/// - No prior head: anyone may create, and `creator` must be the writer.
/// - A prior head: `creator` must not change. The writer must be the creator, a
///   group admin, a community admin, or any member when the policy is `open`.
///   A writer who is allowed only by the `open` policy may not change `admins`
///   or `policy`.
pub fn authorize_write(
    prior: Option<&UserGroup>,
    incoming: &UserGroup,
    writer: &Writer,
) -> Result<(), UserGroupError> {
    let forbid = |m: &str| Err(UserGroupError::Forbidden(m.to_string()));
    let Some(prior) = prior else {
        return if incoming.creator == writer.pubkey {
            Ok(())
        } else {
            forbid("a new group's creator tag must be its signer")
        };
    };
    if incoming.creator != prior.creator {
        return forbid("the creator of a group cannot change");
    }
    let privileged = writer.community_admin
        || writer.pubkey == prior.creator
        || prior.admins.contains(&writer.pubkey);
    if privileged {
        return Ok(());
    }
    if prior.policy != EditPolicy::Open {
        return forbid("only the group creator, group admins, and community admins can edit it");
    }
    if incoming.admins != prior.admins || incoming.policy != prior.policy {
        return forbid("only a group admin can change a group's admins or edit policy");
    }
    Ok(())
}

/// The `group:<handle>` marker for a mention `p` tag.
pub fn group_mention_marker(handle: &str) -> String {
    format!("{GROUP_MENTION_MARKER_PREFIX}{handle}")
}

/// The handle in a `p` tag's fourth element, when it is a group marker.
pub fn group_mention_handle(tag: &[String]) -> Option<&str> {
    match tag {
        [name, _, _, marker, ..] if name == "p" => marker.strip_prefix(GROUP_MENTION_MARKER_PREFIX),
        _ => None,
    }
}

/// Mentioned pubkeys per group handle, from the group-marked `p` tags of `event`.
///
/// Pubkeys keep event order. Tags with a malformed handle or pubkey are skipped.
pub fn group_marked_p_tags(event: &Event) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(handle) = group_mention_handle(parts) else {
            continue;
        };
        let (Ok(handle), Ok(pubkey)) = (normalize_handle(handle), normalize_pubkey(&parts[1]))
        else {
            continue;
        };
        out.entry(handle).or_default().push(pubkey);
    }
    out
}

/// How many mentions the push hellthread gate counts for `event`: each plain `p`
/// tag counts once, and each distinct group counts once however many members it
/// names. `skip` excludes tags the caller does not count, such as workflow owner
/// attribution.
pub fn weighted_p_count(event: &Event, skip: impl Fn(&Tag) -> bool) -> u64 {
    let mut plain = 0u64;
    let mut groups = HashSet::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("p") || skip(tag) {
            continue;
        }
        match group_mention_handle(parts) {
            Some(handle) => {
                groups.insert(handle.to_ascii_lowercase());
            }
            None => plain += 1,
        }
    }
    plain + groups.len() as u64
}

/// Check that every group-marked `p` tag of a message names a current member.
///
/// `lookup` returns the active group for a handle. A group that does not exist or is
/// disabled, a pubkey that is not a member, or more than
/// [`MAX_GROUP_MENTIONS_PER_EVENT`] groups is an error.
pub fn validate_group_mentions(
    event: &Event,
    lookup: impl Fn(&str) -> Option<UserGroup>,
) -> Result<(), UserGroupError> {
    let marked = group_marked_p_tags(event);
    if marked.len() > MAX_GROUP_MENTIONS_PER_EVENT {
        return invalid(format!(
            "a message may mention at most {MAX_GROUP_MENTIONS_PER_EVENT} groups"
        ));
    }
    for (handle, pubkeys) in &marked {
        let Some(group) = lookup(handle).filter(|g| !g.disabled) else {
            return invalid(format!("no active user group @{handle}"));
        };
        let members: HashSet<&str> = group.members.iter().map(String::as_str).collect();
        if let Some(outsider) = pubkeys.iter().find(|p| !members.contains(p.as_str())) {
            return invalid(format!(
                "{outsider} is not a member of @{handle}; refresh the group and send again"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    fn pk(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn event(keys: &Keys, tags: Vec<Vec<String>>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(KIND_USER_GROUP as u16), content)
            .tags(tags.into_iter().map(|t| Tag::parse(t).expect("tag")))
            .sign_with_keys(keys)
            .expect("sign")
    }

    fn base(creator: &str) -> Vec<Vec<String>> {
        vec![
            vec!["d".into(), "ios-team".into()],
            vec!["name".into(), "iOS Team".into()],
            vec!["creator".into(), creator.into()],
        ]
    }

    fn with(mut tags: Vec<Vec<String>>, extra: &[&[&str]]) -> Vec<Vec<String>> {
        tags.extend(
            extra
                .iter()
                .map(|t| t.iter().map(|s| s.to_string()).collect()),
        );
        tags
    }

    #[test]
    fn handles_normalize_and_reject() {
        assert_eq!(normalize_handle("  @iOS-Team ").unwrap(), "ios-team");
        assert_eq!(normalize_handle("a.b_c-1").unwrap(), "a.b_c-1");
        for bad in [
            "",
            "@",
            "-lead",
            ".x",
            "has space",
            "emoji😀",
            "a/b",
            "channel",
            "HERE",
            "Everyone",
            "all",
        ] {
            assert!(normalize_handle(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(normalize_handle(&"a".repeat(MAX_HANDLE_LEN)).is_ok());
        assert!(normalize_handle(&"a".repeat(MAX_HANDLE_LEN + 1)).is_err());
    }

    #[test]
    fn parses_a_full_group() {
        let keys = Keys::generate();
        let creator = keys.public_key().to_hex();
        let tags = with(
            base(&creator),
            &[
                &["p", &pk(1)],
                &["p", &pk(2).to_uppercase()],
                &["admin", &pk(3)],
                &["policy", "open"],
                &["status", "disabled"],
            ],
        );
        let group = parse_user_group(&event(&keys, tags, "Builds the iOS app")).unwrap();
        assert_eq!(group.handle, "ios-team");
        assert_eq!(group.name, "iOS Team");
        assert_eq!(group.description, "Builds the iOS app");
        assert_eq!(group.creator, creator);
        assert_eq!(group.members, vec![pk(1), pk(2)]);
        assert_eq!(group.admins, vec![pk(3)]);
        assert_eq!(group.policy, EditPolicy::Open);
        assert!(group.disabled);
    }

    #[test]
    fn rejects_malformed_groups() {
        let keys = Keys::generate();
        let creator = keys.public_key().to_hex();
        let long_name = "n".repeat(MAX_NAME_LEN + 1);
        let many: Vec<Vec<String>> = (0..=MAX_MEMBERS)
            .map(|i| vec!["p".into(), format!("{i:064x}")])
            .collect();
        let cases: Vec<(&str, Vec<Vec<String>>, String, &str)> = vec![
            (
                "no d",
                base(&creator)[1..].to_vec(),
                String::new(),
                "exactly one d",
            ),
            (
                "two d",
                with(base(&creator), &[&["d", "other"]]),
                String::new(),
                "at most one d",
            ),
            (
                "d not normalized",
                vec![
                    vec!["d".into(), "iOS".into()],
                    vec!["name".into(), "x".into()],
                    vec!["creator".into(), creator.clone()],
                ],
                String::new(),
                "normalized handle",
            ),
            (
                "no name",
                vec![base(&creator)[0].clone(), base(&creator)[2].clone()],
                String::new(),
                "exactly one name",
            ),
            (
                "padded name",
                with(
                    vec![base(&creator)[0].clone(), base(&creator)[2].clone()],
                    &[&["name", " x"]],
                ),
                String::new(),
                "padded",
            ),
            (
                "long name",
                with(
                    vec![base(&creator)[0].clone(), base(&creator)[2].clone()],
                    &[&["name", &long_name]],
                ),
                String::new(),
                "at most 128",
            ),
            (
                "no creator",
                base(&creator)[..2].to_vec(),
                String::new(),
                "exactly one creator",
            ),
            (
                "bad creator",
                with(base(&creator)[..2].to_vec(), &[&["creator", "xyz"]]),
                String::new(),
                "64 hex",
            ),
            (
                "h tag",
                with(base(&creator), &[&["h", "chan"]]),
                String::new(),
                "h tag",
            ),
            (
                "duplicate member",
                with(base(&creator), &[&["p", &pk(1)], &["p", &pk(1)]]),
                String::new(),
                "duplicate p",
            ),
            (
                "member with marker",
                with(base(&creator), &[&["p", &pk(1), "", "group:x"]]),
                String::new(),
                "marker",
            ),
            (
                "bad member",
                with(base(&creator), &[&["p", "nope"]]),
                String::new(),
                "64 hex",
            ),
            (
                "too many members",
                with(base(&creator), &[]).into_iter().chain(many).collect(),
                String::new(),
                "more than 500",
            ),
            (
                "bad policy",
                with(base(&creator), &[&["policy", "anyone"]]),
                String::new(),
                "policy must be",
            ),
            (
                "bad status",
                with(base(&creator), &[&["status", "gone"]]),
                String::new(),
                "status must be",
            ),
            (
                "long description",
                base(&creator),
                "d".repeat(MAX_DESCRIPTION_LEN + 1),
                "description exceeds",
            ),
        ];
        for (name, tags, content, needle) in cases {
            match parse_user_group(&event(&keys, tags, &content)) {
                Err(e) => assert!(e.to_string().contains(needle), "{name}: {e}"),
                Ok(_) => panic!("{name}: must be rejected"),
            }
        }
    }

    #[test]
    fn tags_round_trip_through_parse_and_validate() {
        let keys = Keys::generate();
        let group = UserGroup {
            members: vec![pk(1), pk(2)],
            admins: vec![pk(3)],
            policy: EditPolicy::Open,
            disabled: true,
            description: "d".into(),
            ..group(&keys.public_key().to_hex(), &[], EditPolicy::Open)
        };
        group.validate().unwrap();
        let parsed = parse_user_group(&event(&keys, group.tags(), "d")).unwrap();
        assert_eq!(parsed, group);

        let bad = UserGroup {
            handle: "Not Normal".into(),
            ..group
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn rejects_other_kinds() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::TextNote, "")
            .sign_with_keys(&keys)
            .unwrap();
        assert!(parse_user_group(&ev).is_err());
    }

    fn group(creator: &str, admins: &[&str], policy: EditPolicy) -> UserGroup {
        UserGroup {
            handle: "ios-team".into(),
            name: "iOS Team".into(),
            description: String::new(),
            creator: creator.into(),
            admins: admins.iter().map(|s| s.to_string()).collect(),
            members: vec![],
            policy,
            disabled: false,
        }
    }

    fn writer(pubkey: &str, community_admin: bool) -> Writer {
        Writer {
            pubkey: pubkey.into(),
            community_admin,
        }
    }

    #[test]
    fn authorization_table() {
        let (creator, admin, member, boss) = (pk(1), pk(2), pk(3), pk(4));
        let closed = group(&creator, &[&admin], EditPolicy::Admins);
        let open = group(&creator, &[&admin], EditPolicy::Open);
        let edit = |g: &UserGroup| UserGroup {
            name: "New".into(),
            ..g.clone()
        };

        assert!(authorize_write(None, &closed, &writer(&creator, false)).is_ok());
        assert!(authorize_write(None, &closed, &writer(&member, false)).is_err());

        assert!(authorize_write(Some(&closed), &edit(&closed), &writer(&creator, false)).is_ok());
        assert!(authorize_write(Some(&closed), &edit(&closed), &writer(&admin, false)).is_ok());
        assert!(authorize_write(Some(&closed), &edit(&closed), &writer(&boss, true)).is_ok());
        assert!(authorize_write(Some(&closed), &edit(&closed), &writer(&member, false)).is_err());

        assert!(authorize_write(Some(&open), &edit(&open), &writer(&member, false)).is_ok());
        let promote = UserGroup {
            admins: vec![admin.clone(), member.clone()],
            ..open.clone()
        };
        assert!(authorize_write(Some(&open), &promote, &writer(&member, false)).is_err());
        assert!(authorize_write(Some(&open), &promote, &writer(&admin, false)).is_ok());
        let lock = UserGroup {
            policy: EditPolicy::Admins,
            ..open.clone()
        };
        assert!(authorize_write(Some(&open), &lock, &writer(&member, false)).is_err());

        let steal = UserGroup {
            creator: member.clone(),
            ..closed.clone()
        };
        assert!(authorize_write(Some(&closed), &steal, &writer(&boss, true)).is_err());
    }

    #[test]
    fn head_is_newest_then_lowest_id_among_authors() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let make = |keys: &Keys, at: u64, extra: &[&[&str]]| {
            EventBuilder::new(Kind::Custom(KIND_USER_GROUP as u16), "")
                .tags(
                    with(base(&a.public_key().to_hex()), extra)
                        .into_iter()
                        .map(|t| Tag::parse(t).unwrap()),
                )
                .custom_created_at(nostr::Timestamp::from(at))
                .sign_with_keys(keys)
                .unwrap()
        };
        let old = make(&a, 100, &[&["p", &pk(1)]]);
        let new = make(&b, 200, &[&["p", &pk(2)]]);
        let junk = EventBuilder::new(Kind::Custom(KIND_USER_GROUP as u16), "")
            .custom_created_at(nostr::Timestamp::from(300))
            .sign_with_keys(&b)
            .unwrap();
        let events = vec![old, junk, new.clone()];
        let (head, parsed) = resolve_head("ios-team", &events).unwrap();
        assert_eq!(head.id, new.id);
        assert_eq!(parsed.members, vec![pk(2)]);
        assert!(resolve_head("other", &events).is_none());
    }

    fn message(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::Custom(9), "hi @ios-team")
            .tags(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn group_marked_tags_are_collected_per_handle() {
        let ev = message(&[
            &["p", &pk(1), "", "group:ios-team"],
            &["p", &pk(2), "", "group:ios-team"],
            &["p", &pk(3), "", "group:BAD HANDLE"],
            &["p", &pk(4)],
            &["p", &pk(5), "", "owner"],
        ]);
        let marked = group_marked_p_tags(&ev);
        assert_eq!(marked.len(), 1);
        assert_eq!(marked["ios-team"], vec![pk(1), pk(2)]);
        assert_eq!(weighted_p_count(&ev, |_| false), 1 + 1 + 1 + 1);
    }

    #[test]
    fn hellthread_weight_counts_a_group_once() {
        let mut tags: Vec<Vec<String>> = (0..40)
            .map(|i| {
                vec![
                    "p".into(),
                    format!("{i:064x}"),
                    "".into(),
                    "group:big".into(),
                ]
            })
            .collect();
        tags.push(vec!["p".into(), pk(200)]);
        let refs: Vec<Vec<&str>> = tags
            .iter()
            .map(|t| t.iter().map(String::as_str).collect())
            .collect();
        let slices: Vec<&[&str]> = refs.iter().map(Vec::as_slice).collect();
        let ev = message(&slices);
        assert_eq!(weighted_p_count(&ev, |_| false), 2);
    }

    #[test]
    fn mentions_must_name_current_members_of_an_active_group() {
        let mut g = group(&pk(9), &[], EditPolicy::Admins);
        g.members = vec![pk(1), pk(2)];
        let lookup = |h: &str| (h == "ios-team").then(|| g.clone());

        let ok = message(&[
            &["p", &pk(1), "", "group:ios-team"],
            &["p", &pk(2), "", "group:ios-team"],
            &["p", &pk(7)],
        ]);
        assert!(validate_group_mentions(&ok, lookup).is_ok());

        let outsider = message(&[&["p", &pk(7), "", "group:ios-team"]]);
        assert!(validate_group_mentions(&outsider, lookup)
            .unwrap_err()
            .to_string()
            .contains("not a member"));

        let unknown = message(&[&["p", &pk(1), "", "group:ghost"]]);
        assert!(validate_group_mentions(&unknown, lookup)
            .unwrap_err()
            .to_string()
            .contains("no active user group"));

        let mut off = g.clone();
        off.disabled = true;
        let disabled = |_: &str| Some(off.clone());
        assert!(validate_group_mentions(&ok, disabled).is_err());

        let six: Vec<Vec<String>> = (0..=MAX_GROUP_MENTIONS_PER_EVENT)
            .map(|i| vec!["p".into(), pk(1), "".into(), format!("group:g{i}")])
            .collect();
        let refs: Vec<Vec<&str>> = six
            .iter()
            .map(|t| t.iter().map(String::as_str).collect())
            .collect();
        let slices: Vec<&[&str]> = refs.iter().map(Vec::as_slice).collect();
        assert!(
            validate_group_mentions(&message(&slices), |_| Some(g.clone()))
                .unwrap_err()
                .to_string()
                .contains("at most 5")
        );
    }
}
