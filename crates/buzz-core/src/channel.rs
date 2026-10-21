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

/// Maximum length of a canonical channel name, in characters. Matches
/// `channels.name VARCHAR(255)`, so the relay can refuse a longer name before
/// it stores the command event instead of failing when it applies it.
pub const MAX_CHANNEL_NAME_CHARS: usize = 255;

/// Whether `name` is a storable channel name: nonempty and at most
/// [`MAX_CHANNEL_NAME_CHARS`] characters after [`canonical_channel_name`].
pub fn channel_name_error(name: &str) -> Option<&'static str> {
    let name = canonical_channel_name(name);
    if name.is_empty() {
        Some("channel name is required")
    } else if name.chars().count() > MAX_CHANNEL_NAME_CHARS {
        Some("channel name is longer than 255 characters")
    } else {
        None
    }
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
/// `""` alone (one or more times) means "no labels" and returns an empty
/// set; kind:9002 uses it to clear the labels, the same convention as
/// `["ttl", ""]`. Repeated labels are stored once and count once toward
/// [`MAX_CHANNEL_LABELS`].
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
/// signs (kinds 39000–39002), in this order:
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

/// The role a person gets in a channel when nobody picks one
/// (`channels.join_role`): a self-join (kind:9021, or a kind:9000 self-add),
/// or a kind:9000 add with no `role` tag.
///
/// This never changes what a role can do. [`MemberRole`] permissions are the
/// same in every channel; this setting only decides admission. An open channel
/// with `Guest` is an **announce channel**: anyone can follow and read it, but
/// only people a member, admin or owner makes a member can write. In NIP-29
/// terms, `Guest` on an open channel is the `restricted` flag of kind:39000.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelJoinRole {
    /// New people become members. The default, and today's behavior.
    Member,
    /// New people become guests: they can read and leave, not write.
    Guest,
}

impl ChannelJoinRole {
    /// Canonical string representation (matches the DB column and Nostr tags).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Guest => "guest",
        }
    }

    /// Parse a stored value for an authorization decision. An unknown value
    /// is treated as [`ChannelJoinRole::Guest`], so it can never widen access.
    pub fn parse_fail_closed(value: &str) -> Self {
        value.parse().unwrap_or(Self::Guest)
    }

    /// The [`MemberRole`] this setting gives a new person.
    pub fn member_role(&self) -> MemberRole {
        match self {
            Self::Member => MemberRole::Member,
            Self::Guest => MemberRole::Guest,
        }
    }

    /// Whether a non-member who adds themselves may ask for `requested`.
    /// A joiner may ask for `join_role` or a lower role, never a higher one.
    /// `Bot` has member permissions, so it counts as `Member` here.
    pub fn allows_self_join_as(&self, requested: MemberRole) -> bool {
        match requested {
            MemberRole::Guest => true,
            MemberRole::Member | MemberRole::Bot => *self == Self::Member,
            MemberRole::Owner | MemberRole::Admin => false,
        }
    }
}

impl fmt::Display for ChannelJoinRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChannelJoinRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "member" => Ok(Self::Member),
            "guest" => Ok(Self::Guest),
            other => Err(format!("unknown channel join role: {other:?}")),
        }
    }
}

/// The facts a kind:9000 add or role change is decided on, for
/// [`decide_member_put`]. Every field must come from one consistent read: the
/// relay validator reads them for a fast, client-visible answer, and
/// `add_member` reads them again under the channel's membership lock, so the
/// stored role is always decided on current state.
#[derive(Debug, Clone, Copy)]
pub struct MemberPut<'a> {
    /// Whether the channel is private.
    pub private: bool,
    /// The channel's `join_role`.
    pub join_role: ChannelJoinRole,
    /// The actor's active role, or `None` when the actor is not a member.
    pub actor_role: Option<MemberRole>,
    /// Whether the actor is adding themselves.
    pub target_is_actor: bool,
    /// The target's active stored role, or `None` when the target is not an
    /// active member. A soft-removed row is not an active role.
    pub target_role: Option<&'a str>,
    /// The role the event asks for, or `None` when it has no `role` tag.
    pub requested: Option<MemberRole>,
}

/// Why [`decide_member_put`] refused an add or role change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberPutDenial {
    /// A non-member tried to add someone to a private channel.
    NotAMember,
    /// A non-elevated actor asked for owner or admin.
    ElevatedRoleGrant,
    /// A guest, or a non-member where joining does not give `member`, tried
    /// to add someone else. Adding people is a write.
    GuestCannotAdd,
    /// A non-member adding themselves asked for a role above `join_role`.
    SelfJoinAboveJoinRole,
    /// The actor may not change the target's active role.
    RoleChange,
}

/// Decide a kind:9000 add or role change, and the role to store.
///
/// This is the one definition of the admission rules: a guest never adds
/// anyone; a non-member adds others only where joining gives `member`; a
/// joiner gets `join_role` or a lower role it asks for; only owners and admins
/// grant elevated roles; and an active role changes only as
/// [`MemberRole::may_change_role`] allows. With no `role` tag, an active member
/// keeps their role and a new member gets `join_role`.
///
/// Returns the role to store. The last-owner rule needs the owner count and
/// the target agent's `channel_add_policy` needs its own read; callers check
/// both separately.
pub fn decide_member_put(put: MemberPut<'_>) -> Result<MemberRole, MemberPutDenial> {
    let actor_elevated = put.actor_role.is_some_and(|role| role.is_elevated());
    if put.private && put.actor_role.is_none() {
        return Err(MemberPutDenial::NotAMember);
    }
    if put.target_is_actor && put.actor_role.is_none() {
        // A joiner (open channels only; private ones are refused above).
        if put
            .requested
            .is_some_and(|role| !put.join_role.allows_self_join_as(role))
        {
            return Err(MemberPutDenial::SelfJoinAboveJoinRole);
        }
    } else if put.target_role.is_none()
        && put.requested.is_some_and(|role| role.is_elevated())
        && !actor_elevated
    {
        // Granting an elevated role to a new member. An active member's role
        // change is decided by `may_change_role` below.
        return Err(MemberPutDenial::ElevatedRoleGrant);
    }
    if !put.target_is_actor {
        let may_add = match put.actor_role {
            Some(role) => role.can_post(),
            None => put.join_role == ChannelJoinRole::Member,
        };
        if !may_add {
            return Err(MemberPutDenial::GuestCannotAdd);
        }
    }

    let Some(current) = put.target_role else {
        return Ok(put.requested.unwrap_or(put.join_role.member_role()));
    };
    let Some(requested) = put.requested else {
        // No role tag: an active member keeps their role.
        return current.parse().map_err(|_| MemberPutDenial::RoleChange);
    };
    if current == requested.as_str() {
        return Ok(requested);
    }
    // A stored role this build does not know is changed only by an elevated
    // actor.
    match current.parse::<MemberRole>() {
        Ok(current) if MemberRole::may_change_role(put.actor_role, current, requested) => {
            Ok(requested)
        }
        Ok(_) => Err(MemberPutDenial::RoleChange),
        Err(_) if actor_elevated => Ok(requested),
        Err(_) => Err(MemberPutDenial::RoleChange),
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
    /// Access-control channel. The relay leaves it out of queries unless a
    /// filter names it, or asks for system channels with `#t:["system"]` on
    /// kinds 39000-39003.
    System,
}

/// The channel types that every client knew before NIP-CL added new ones.
///
/// A client sees a channel only if it knows the channel's type. Queries that
/// do not ask for a type therefore return only these types. Every other type,
/// including any type added later, stays hidden unless a filter asks for it.
///
/// It is also the rule for membership notices (kind 44100/44101): a filter
/// without `#t` sees notices only about these types.
///
/// Frozen. Do not add a type here: a new type must stay hidden from clients
/// that do not know it, so it needs its own opt-in.
pub const LEGACY_CHANNEL_TYPES: [&str; 4] = ["stream", "forum", "dm", "workflow"];

impl ChannelType {
    /// Canonical string representation (matches DB enum and Nostr tags).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Forum => "forum",
            Self::Dm => "dm",
            Self::Workflow => "workflow",
            Self::System => "system",
        }
    }

    /// Visibility of a new channel of this type when its create event has no
    /// `visibility` tag. A system channel holds access-control data, so it is
    /// private unless its creator asks for open.
    pub fn default_visibility(&self) -> ChannelVisibility {
        match self {
            Self::System => ChannelVisibility::Private,
            Self::Stream | Self::Forum | Self::Dm | Self::Workflow => ChannelVisibility::Open,
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
            "system" => Ok(Self::System),
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

    /// Whether this role may write in a channel: post, react, edit, change
    /// topic, add people, push to git. The same in every channel: every role
    /// except [`MemberRole::Guest`] may, and a bot counts as a member.
    pub fn can_post(&self) -> bool {
        !matches!(self, Self::Guest)
    }

    /// Whether an actor whose active role is `actor` may change another
    /// active member's role from `current` to `requested`.
    ///
    /// Owners and admins may make any change (the last-owner rule is checked
    /// separately). Any other active member who may post may make a guest a
    /// member: the same authority they have to add a new member. This holds
    /// for agents too; a later restricted-agent level may limit it. Nobody
    /// else may change a role; a guest cannot promote anyone, including
    /// themselves.
    pub fn may_change_role(
        actor: Option<MemberRole>,
        current: MemberRole,
        requested: MemberRole,
    ) -> bool {
        let Some(actor) = actor else {
            return false;
        };
        actor.is_elevated()
            || (actor.can_post()
                && current == Self::Guest
                && matches!(requested, Self::Member | Self::Bot))
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
        canonical_channel_name, channel_name_error, channel_type_from_group_state_tags,
        group_state_identity_tags, parse_channel_labels, ChannelJoinRole, ChannelLabelError,
        ChannelType, MemberRole, LEGACY_CHANNEL_TYPES, MAX_CHANNEL_LABELS, RESERVED_CHANNEL_LABELS,
    };

    #[test]
    fn channel_name_error_matches_the_column_limit() {
        assert_eq!(
            channel_name_error("  ## "),
            Some("channel name is required")
        );
        assert_eq!(channel_name_error(&"é".repeat(255)), None);
        assert_eq!(channel_name_error(&format!("# {}", "a".repeat(255))), None);
        assert!(channel_name_error(&"a".repeat(256)).is_some());
    }

    #[test]
    fn only_the_types_from_before_nip_cl_are_legacy() {
        for name in LEGACY_CHANNEL_TYPES {
            let _: ChannelType = name.parse().expect("legacy type parses");
        }
        assert!(!LEGACY_CHANNEL_TYPES.contains(&ChannelType::System.as_str()));
    }

    /// Every channel type name is reserved, so a label cannot pose as a type.
    /// The `match` has no wildcard: a new `ChannelType` variant does not
    /// compile until it is added to `ALL` here, and then this test fails
    /// until its name is in `RESERVED_CHANNEL_LABELS`.
    #[test]
    fn every_channel_type_name_is_a_reserved_label() {
        const ALL: [ChannelType; 5] = [
            ChannelType::Stream,
            ChannelType::Forum,
            ChannelType::Dm,
            ChannelType::Workflow,
            ChannelType::System,
        ];
        for channel_type in ALL {
            match channel_type {
                ChannelType::Stream
                | ChannelType::Forum
                | ChannelType::Dm
                | ChannelType::Workflow
                | ChannelType::System => {}
            }
            assert!(
                RESERVED_CHANNEL_LABELS.contains(&channel_type.as_str()),
                "{channel_type:?} is not a reserved label"
            );
        }
    }

    #[test]
    fn desktop_solo_creation_preserves_mobile_identity_contract() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../test-fixtures/solo-creation.json"))
                .expect("Solo creation fixture");
        let tags: Vec<Vec<String>> = serde_json::from_value(fixture["creation"]["tags"].clone())
            .expect("captured production command tags");
        let labels = super::parse_channel_labels(
            tags.iter()
                .filter(|tag| tag[0] == "t")
                .map(|tag| tag[1].as_str()),
        )
        .expect("valid creation labels");
        let channel_type = tags
            .iter()
            .find(|tag| tag[0] == "channel_type")
            .expect("creation type")[1]
            .as_str();
        let creator =
            hex::decode(fixture["creator"].as_str().expect("creator")).expect("creator hex");
        let actual: Vec<Vec<String>> = group_state_identity_tags(channel_type, &labels, &creator)
            .expect("relay identity tags")
            .into_iter()
            .map(|tag| tag.to_vec())
            .collect();
        assert_eq!(
            serde_json::to_value(actual).expect("identity JSON"),
            fixture["identity_tags"]
        );
    }

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
        // Nine tags with one repeat are 8 distinct labels.
        let mut repeated: Vec<&str> = names[..MAX_CHANNEL_LABELS]
            .iter()
            .map(String::as_str)
            .collect();
        repeated.push(repeated[0]);
        assert_eq!(
            parse_channel_labels(repeated).map(|labels| labels.len()),
            Ok(MAX_CHANNEL_LABELS)
        );
    }

    #[test]
    fn empty_labels_clear_and_cannot_mix_with_labels() {
        assert_eq!(parse_channel_labels([""]), Ok(Vec::new()));
        assert_eq!(parse_channel_labels(["", ""]), Ok(Vec::new()));
        assert_eq!(
            parse_channel_labels(["", "workspace"]),
            Err(ChannelLabelError::ClearWithOthers)
        );
    }

    #[test]
    fn join_role_parses_known_values_and_fails_closed() {
        assert_eq!("member".parse(), Ok(ChannelJoinRole::Member));
        assert_eq!("guest".parse(), Ok(ChannelJoinRole::Guest));
        assert!("announce".parse::<ChannelJoinRole>().is_err());
        assert_eq!(
            ChannelJoinRole::parse_fail_closed("member"),
            ChannelJoinRole::Member
        );
        for unknown in ["", "Member", "everyone", "bot", "member "] {
            assert_eq!(
                ChannelJoinRole::parse_fail_closed(unknown),
                ChannelJoinRole::Guest,
                "{unknown:?}"
            );
        }
        assert_eq!(ChannelJoinRole::Member.member_role(), MemberRole::Member);
        assert_eq!(ChannelJoinRole::Guest.member_role(), MemberRole::Guest);
    }

    #[test]
    fn a_joiner_may_ask_for_join_role_or_lower() {
        use MemberRole::*;
        for (join_role, requested, allowed) in [
            (ChannelJoinRole::Member, Guest, true),
            (ChannelJoinRole::Member, Member, true),
            (ChannelJoinRole::Member, Bot, true),
            (ChannelJoinRole::Member, Admin, false),
            (ChannelJoinRole::Member, Owner, false),
            (ChannelJoinRole::Guest, Guest, true),
            (ChannelJoinRole::Guest, Member, false),
            (ChannelJoinRole::Guest, Bot, false),
            (ChannelJoinRole::Guest, Admin, false),
            (ChannelJoinRole::Guest, Owner, false),
        ] {
            assert_eq!(
                join_role.allows_self_join_as(requested),
                allowed,
                "{join_role} {requested}"
            );
        }
    }

    #[test]
    fn only_guests_cannot_post() {
        for role in [
            MemberRole::Owner,
            MemberRole::Admin,
            MemberRole::Member,
            MemberRole::Bot,
        ] {
            assert!(role.can_post(), "{role}");
        }
        assert!(!MemberRole::Guest.can_post());
    }

    #[test]
    fn only_elevated_roles_change_roles_except_promoting_a_guest() {
        use MemberRole::*;
        const ALL: [MemberRole; 5] = [Owner, Admin, Member, Guest, Bot];
        for actor in [Owner, Admin] {
            for current in ALL {
                for requested in ALL {
                    assert!(MemberRole::may_change_role(Some(actor), current, requested));
                }
            }
        }
        for actor in [Member, Bot] {
            for current in ALL {
                for requested in ALL {
                    let promotes_guest = current == Guest && matches!(requested, Member | Bot);
                    assert_eq!(
                        MemberRole::may_change_role(Some(actor), current, requested),
                        promotes_guest,
                        "{actor}: {current} -> {requested}"
                    );
                }
            }
        }
        for actor in [None, Some(Guest)] {
            for current in ALL {
                for requested in ALL {
                    assert!(!MemberRole::may_change_role(actor, current, requested));
                }
            }
        }
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
