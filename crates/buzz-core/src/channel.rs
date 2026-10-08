//! Channel and membership enums shared across crates.
//!
//! These live in `buzz-core` (zero I/O deps) so both the SDK (client-side)
//! and the DB layer (server-side) can use the same types without pulling in
//! sqlx/tokio.

use std::fmt;
use std::str::FromStr;

/// Returns the canonical display name for a channel.
///
/// Channel names are rendered with a leading `#` by clients, so surrounding
/// whitespace and user-supplied hash prefixes are removed here to keep the
/// stored name prefix-free.
pub fn canonical_channel_name(name: &str) -> &str {
    name.trim_start_matches(|c: char| c == '#' || c.is_whitespace())
        .trim_end()
}

/// Maximum number of labels on one channel.
pub const MAX_CHANNEL_LABELS: usize = 8;

/// Maximum length of one channel label, in bytes.
pub const MAX_CHANNEL_LABEL_LEN: usize = 64;

/// Names a label may not take, because the channel type uses the same `t`
/// tag. A label equal to one of these could make a channel look like
/// another type. The list includes types that a later relay may add.
pub const RESERVED_CHANNEL_LABELS: &[&str] = &["stream", "forum", "dm", "workflow", "system"];

/// Why a set of channel labels was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelLabelError {
    /// More than [`MAX_CHANNEL_LABELS`] distinct labels.
    #[error("too many labels: at most {MAX_CHANNEL_LABELS} are allowed")]
    TooMany,
    /// A label is not 1–64 characters from `a-z`, `0-9`, `.`, `:` and `-`.
    #[error("invalid label {0:?}: use 1-{MAX_CHANNEL_LABEL_LEN} characters from a-z, 0-9, '.', ':' and '-'")]
    Invalid(String),
    /// A label equals a channel type name.
    #[error("invalid label {0:?}: a label cannot be a channel type name")]
    Reserved(String),
    /// The clear marker `""` was combined with other labels.
    #[error("an empty label clears all labels and cannot be combined with other labels")]
    ClearWithOthers,
}

/// Validate the values of a channel's `["t", <label>]` tags and return the
/// label set to store, in first-seen order without duplicates.
///
/// A single `""` means "no labels" and returns an empty set; kind:9002 uses
/// it to clear the labels, the same convention as `["ttl", ""]`.
pub fn parse_channel_labels<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, ChannelLabelError> {
    let values: Vec<&str> = values.into_iter().collect();
    if values.contains(&"") {
        return if values.iter().all(|value| value.is_empty()) {
            Ok(Vec::new())
        } else {
            Err(ChannelLabelError::ClearWithOthers)
        };
    }
    let mut labels: Vec<String> = Vec::with_capacity(values.len());
    for value in values {
        let valid = value.len() <= MAX_CHANNEL_LABEL_LEN
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".:-".contains(&byte)
            });
        if !valid {
            return Err(ChannelLabelError::Invalid(value.to_owned()));
        }
        if RESERVED_CHANNEL_LABELS.contains(&value) {
            return Err(ChannelLabelError::Reserved(value.to_owned()));
        }
        if !labels.iter().any(|label| label == value) {
            labels.push(value.to_owned());
        }
    }
    if labels.len() > MAX_CHANNEL_LABELS {
        return Err(ChannelLabelError::TooMany);
    }
    Ok(labels)
}

/// Tags that identify a channel on every NIP-29 group-state event the relay
/// signs (kinds 39000–39003), in this order:
///
/// 1. `["t", <channel_type>]`. It comes first, so a client that reads the
///    first `t` tag gets the channel type.
/// 2. `["t", <label>]` for each label, in stored order.
/// 3. `["P", <creator hex>]`: the key that signed the channel's kind:9007.
///    It comes from the database and does not change when ownership moves.
///    Uppercase `P` names the author of the root object, as in NIP-22,
///    NIP-34 and NIP-72; lowercase `p` already means members and DM
///    participants on these kinds.
///
/// Filters such as `{kinds:[39000], #P:[<key>], #t:[<label>]}` select by
/// these tags.
pub fn group_state_identity_tags(
    channel_type: &str,
    labels: &[String],
    created_by: &[u8],
) -> Result<Vec<nostr::Tag>, nostr::event::tag::Error> {
    let mut tags = Vec::with_capacity(labels.len() + 2);
    tags.push(nostr::Tag::parse(["t", channel_type])?);
    for label in labels {
        tags.push(nostr::Tag::parse(["t", label.as_str()])?);
    }
    tags.push(nostr::Tag::parse(["P", &hex::encode(created_by)])?);
    Ok(tags)
}

/// Return the channel type from group-state tags: the value of the first
/// `t` tag. Later `t` tags are labels.
pub fn channel_type_from_group_state_tags<'a>(
    tags: impl IntoIterator<Item = &'a nostr::Tag>,
) -> Option<&'a str> {
    tags.into_iter()
        .find(|tag| tag.kind() == nostr::TagKind::t())
        .and_then(|tag| tag.content())
}

/// Tag on kind:39000 that holds the channel's creation time in Unix seconds.
///
/// The event's own `created_at` is the time of the last edit, so it cannot
/// rank channels by age.
pub const CHANNEL_CREATED_AT_TAG: &str = "created_at";

/// The [`CHANNEL_CREATED_AT_TAG`] tag for a channel created at `unix_seconds`.
pub fn channel_created_at_tag(unix_seconds: i64) -> Result<nostr::Tag, nostr::event::tag::Error> {
    nostr::Tag::parse([CHANNEL_CREATED_AT_TAG, &unix_seconds.to_string()])
}

/// The identity tags of a relay-signed group-state event, as written by
/// [`group_state_identity_tags`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupStateIdentity {
    /// The first `t` value: the channel type.
    pub channel_type: Option<String>,
    /// Every later `t` value, in tag order: the channel labels.
    pub labels: Vec<String>,
    /// The first `P` value: the hex key that created the channel. Trust it
    /// only when the relay key signed the event.
    pub created_by: Option<String>,
}

impl GroupStateIdentity {
    /// Read the channel type, labels and creator from `(name, value)` tag
    /// pairs. Tags without a value are skipped.
    pub fn from_tag_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut identity = Self::default();
        for (name, value) in pairs {
            match name {
                "t" if identity.channel_type.is_none() => {
                    identity.channel_type = Some(value.to_owned());
                }
                "t" => identity.labels.push(value.to_owned()),
                "P" if identity.created_by.is_none() => {
                    identity.created_by = Some(value.to_owned());
                }
                _ => {}
            }
        }
        identity
    }

    /// Read the channel type, labels and creator from Nostr tags.
    pub fn from_tags<'a>(tags: impl IntoIterator<Item = &'a nostr::Tag>) -> Self {
        Self::from_tag_pairs(tags.into_iter().filter_map(|tag| {
            let slice = tag.as_slice();
            Some((slice.first()?.as_str(), slice.get(1)?.as_str()))
        }))
    }
}

/// Whether a channel is publicly visible or invite-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelVisibility {
    /// Searchable; anyone can join without an invite.
    Open,
    /// Hidden; requires an invite to join.
    Private,
}

impl ChannelVisibility {
    /// Canonical string representation (matches DB enum and Nostr tags).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Private => "private",
        }
    }
}

impl fmt::Display for ChannelVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChannelVisibility {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Self::Open),
            "private" => Ok(Self::Private),
            other => Err(format!("unknown channel visibility: {other:?}")),
        }
    }
}

/// The functional type of a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    /// Linear message stream (the default).
    Stream,
    /// Threaded forum-style discussion.
    Forum,
    /// Direct message conversation.
    Dm,
    /// Internal workflow execution channel.
    Workflow,
}

impl ChannelType {
    /// Canonical string representation (matches DB enum and Nostr tags).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Forum => "forum",
            Self::Dm => "dm",
            Self::Workflow => "workflow",
        }
    }
}

impl fmt::Display for ChannelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChannelType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stream" => Ok(Self::Stream),
            "forum" => Ok(Self::Forum),
            "dm" => Ok(Self::Dm),
            "workflow" => Ok(Self::Workflow),
            other => Err(format!("unknown channel type: {other:?}")),
        }
    }
}

/// A member's role within a channel.
///
/// The hierarchy for permission checks is: Owner > Admin > Member > Guest.
/// Bot is a **separate designation** — it is not part of the linear hierarchy.
/// Use [`MemberRole::permission_level`] for numeric comparisons in authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberRole {
    /// Full control — can manage members and delete the channel.
    Owner,
    /// Can manage members and channel settings.
    Admin,
    /// Standard participant.
    Member,
    /// Read-only external participant.
    Guest,
    /// Automated agent or integration (not in the role hierarchy).
    Bot,
}

impl MemberRole {
    /// Canonical string representation (matches DB enum and Nostr tags).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Guest => "guest",
            Self::Bot => "bot",
        }
    }

    /// Elevated roles that only existing owners/admins may grant.
    pub fn is_elevated(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Numeric permission level for authorization comparisons.
    ///
    /// Higher = more privileged. Bot returns 0 (must use explicit grants).
    /// Use `role.permission_level() >= required.permission_level()` for checks.
    pub fn permission_level(self) -> u8 {
        match self {
            Self::Owner => 4,
            Self::Admin => 3,
            Self::Member => 2,
            Self::Guest => 1,
            Self::Bot => 0,
        }
    }

    /// Returns true if this role meets or exceeds the required role's permission level.
    ///
    /// Bot never meets any requirement (returns false for all non-Bot requirements).
    pub fn has_at_least(self, required: MemberRole) -> bool {
        self.permission_level() >= required.permission_level()
    }
}

impl fmt::Display for MemberRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemberRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            "guest" => Ok(Self::Guest),
            "bot" => Ok(Self::Bot),
            other => Err(format!("unknown member role: {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_channel_name, channel_type_from_group_state_tags, group_state_identity_tags,
        parse_channel_labels, ChannelLabelError, GroupStateIdentity, MAX_CHANNEL_LABELS,
    };

    #[test]
    fn group_state_identity_tags_put_the_type_first_then_labels_then_the_creator() {
        let creator = [0xab; 32];
        let labels = vec!["workspace".to_string(), "team:core".to_string()];
        let built = group_state_identity_tags("forum", &labels, &creator).expect("valid tags");
        assert_eq!(channel_type_from_group_state_tags(&built), Some("forum"));
        let tags: Vec<Vec<String>> = built.into_iter().map(|tag| tag.to_vec()).collect();
        assert_eq!(
            tags,
            vec![
                vec!["t".to_string(), "forum".to_string()],
                vec!["t".to_string(), "workspace".to_string()],
                vec!["t".to_string(), "team:core".to_string()],
                vec!["P".to_string(), "ab".repeat(32)],
            ]
        );
    }

    #[test]
    fn group_state_identity_reads_back_what_the_relay_writes() {
        let creator = [0xab; 32];
        let labels = vec!["workspace".to_string(), "team:core".to_string()];
        let mut tags = vec![nostr::Tag::parse(["d", "channel"]).unwrap()];
        tags.extend(group_state_identity_tags("forum", &labels, &creator).unwrap());
        tags.push(nostr::Tag::parse(["p", &"cd".repeat(32), "", "member"]).unwrap());
        assert_eq!(
            GroupStateIdentity::from_tags(&tags),
            GroupStateIdentity {
                channel_type: Some("forum".to_string()),
                labels,
                created_by: Some("ab".repeat(32)),
            }
        );
        assert_eq!(
            GroupStateIdentity::from_tag_pairs([("name", "x")]),
            GroupStateIdentity::default()
        );
    }

    #[test]
    fn channel_labels_accept_the_allowed_alphabet_and_drop_duplicates() {
        assert_eq!(
            parse_channel_labels(["agent-attention", "v1.2:x", "agent-attention"]),
            Ok(vec!["agent-attention".to_string(), "v1.2:x".to_string()])
        );
        assert_eq!(
            parse_channel_labels(["a".repeat(64).as_str()]),
            Ok(vec!["a".repeat(64)])
        );
    }

    #[test]
    fn channel_labels_refuse_bad_characters_lengths_and_type_names() {
        for bad in ["Upper", "has space", "under_score", "emoji🙂"] {
            assert_eq!(
                parse_channel_labels([bad]),
                Err(ChannelLabelError::Invalid(bad.to_string()))
            );
        }
        let long = "a".repeat(65);
        assert_eq!(
            parse_channel_labels([long.as_str()]),
            Err(ChannelLabelError::Invalid(long.clone()))
        );
        for reserved in ["stream", "forum", "dm", "workflow", "system"] {
            assert_eq!(
                parse_channel_labels([reserved]),
                Err(ChannelLabelError::Reserved(reserved.to_string()))
            );
        }
    }

    #[test]
    fn channel_labels_cap_the_distinct_count() {
        let names: Vec<String> = (0..=MAX_CHANNEL_LABELS).map(|i| format!("l{i}")).collect();
        assert_eq!(
            parse_channel_labels(names.iter().map(String::as_str)),
            Err(ChannelLabelError::TooMany)
        );
        assert_eq!(
            parse_channel_labels(names[..MAX_CHANNEL_LABELS].iter().map(String::as_str))
                .map(|labels| labels.len()),
            Ok(MAX_CHANNEL_LABELS)
        );
    }

    #[test]
    fn a_lone_empty_label_clears_and_cannot_mix_with_labels() {
        assert_eq!(parse_channel_labels([""]), Ok(Vec::new()));
        assert_eq!(
            parse_channel_labels(["", "workspace"]),
            Err(ChannelLabelError::ClearWithOthers)
        );
    }

    #[test]
    fn channel_names_trim_whitespace_and_drop_all_leading_hashes() {
        assert_eq!(canonical_channel_name("channel"), "channel");
        assert_eq!(canonical_channel_name("#channel"), "channel");
        assert_eq!(canonical_channel_name("###channel"), "channel");
        assert_eq!(canonical_channel_name("  ###channel  "), "channel");
        assert_eq!(canonical_channel_name("# channel"), "channel");
        assert_eq!(canonical_channel_name("### channel  "), "channel");
        assert_eq!(canonical_channel_name("  ###  "), "");
        assert_eq!(canonical_channel_name("# #"), "");
        assert_eq!(canonical_channel_name("### ###"), "");
        assert_eq!(canonical_channel_name("channel#topic"), "channel#topic");
    }
}
