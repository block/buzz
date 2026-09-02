//! Project related-channel command envelopes.

use buzz_core::kind::{
    KIND_PROJECT, KIND_PROJECT_RELATED_CHANNEL, KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT,
};
use nostr::{Event, EventBuilder, EventId, Kind, PublicKey, Tag};
use uuid::Uuid;

use crate::SdkError;

/// Maximum byte length of the Project `d` segment in a command coordinate.
pub const PROJECT_RELATED_CHANNEL_D_MAX_LEN: usize = 1024;

/// Maximum number of effective related channels in one relay snapshot.
pub use buzz_core::project_related_channels::PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP;

/// Canonical Project coordinate used by related-channel commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRelatedChannelCoordinate {
    /// Canonical `30621:<owner>:<d>` coordinate.
    pub coordinate: String,
    /// Project owner public key.
    pub owner: PublicKey,
    /// Project `d` identifier.
    pub project_d: String,
}

impl ProjectRelatedChannelCoordinate {
    /// Parse a canonical `30621:<lowercase-owner>:<non-empty-d>` coordinate.
    pub fn parse(value: &str) -> Result<Self, SdkError> {
        let (owner, project_d) = parse_project_coordinate_parts(value)?;
        Ok(Self {
            coordinate: value.to_owned(),
            owner,
            project_d,
        })
    }
}

/// One related-channel state transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectRelatedChannelOperation {
    /// Link the channel.
    Add,
    /// Unlink the channel.
    Remove,
}

impl ProjectRelatedChannelOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
        }
    }
}

/// Strictly parsed `kind:47010` Project related-channel command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRelatedChannelCommand {
    /// Canonical `30621:<owner>:<d>` Project coordinate.
    pub project_coordinate: String,
    /// Project owner from the coordinate.
    pub project_owner: PublicKey,
    /// Project identifier from the coordinate.
    pub project_d: String,
    /// Related channel changed by this command.
    pub channel_id: Uuid,
    /// Requested state transition.
    pub operation: ProjectRelatedChannelOperation,
}

fn parse_project_coordinate_parts(value: &str) -> Result<(PublicKey, String), SdkError> {
    let mut parts = value.splitn(3, ':');
    let kind = parts.next().unwrap_or_default();
    let owner = parts.next().unwrap_or_default();
    let project_d = parts.next().unwrap_or_default();
    if kind != KIND_PROJECT.to_string() {
        return Err(SdkError::InvalidInput(format!(
            "Project coordinate must start with `{KIND_PROJECT}:`"
        )));
    }
    if owner.len() != 64
        || !owner
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(SdkError::InvalidInput(
            "Project coordinate owner must be 64 lowercase hex characters".into(),
        ));
    }
    if project_d.is_empty() || project_d.len() > PROJECT_RELATED_CHANNEL_D_MAX_LEN {
        return Err(SdkError::InvalidInput(format!(
            "Project coordinate d must be 1..={PROJECT_RELATED_CHANNEL_D_MAX_LEN} bytes"
        )));
    }
    let owner = PublicKey::from_hex(owner)
        .map_err(|_| SdkError::InvalidInput("Project coordinate owner is invalid".into()))?;
    Ok((owner, project_d.to_owned()))
}

fn canonical_event_id(value: &str) -> Result<String, SdkError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(SdkError::InvalidInput(
            "Project event id must be 64 lowercase hex characters".into(),
        ));
    }
    EventId::from_hex(value)
        .map(|event_id| event_id.to_hex())
        .map_err(|_| {
            SdkError::InvalidInput("Project event id must be 64 lowercase hex characters".into())
        })
}

/// Parse and validate a complete related-channel command envelope.
///
/// Unknown tags are rejected. The sole exception is one ambient four-element
/// NIP-OA `auth` tag, which transports owner delegation without changing the
/// command itself.
pub fn parse_project_related_channel_command(
    event: &Event,
) -> Result<ProjectRelatedChannelCommand, SdkError> {
    if u32::from(event.kind.as_u16()) != KIND_PROJECT_RELATED_CHANNEL {
        return Err(SdkError::InvalidInput(format!(
            "expected kind {KIND_PROJECT_RELATED_CHANNEL}"
        )));
    }
    if !event.content.is_empty() {
        return Err(SdkError::InvalidInput(
            "Project related-channel command content must be empty".into(),
        ));
    }

    let mut project_coordinate = None;
    let mut operation = None;
    let mut channel_id = None;
    let mut auth_seen = false;

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let name = parts.first().map(String::as_str).unwrap_or_default();
        let require_pair = || {
            if parts.len() == 2 {
                Ok(parts[1].as_str())
            } else {
                Err(SdkError::InvalidInput(format!(
                    "`{name}` tag must have exactly two elements"
                )))
            }
        };
        match name {
            "a" => {
                if project_coordinate
                    .replace(require_pair()?.to_owned())
                    .is_some()
                {
                    return Err(SdkError::InvalidInput("duplicate `a` tag".into()));
                }
            }
            "op" => {
                let parsed = match require_pair()? {
                    "add" => ProjectRelatedChannelOperation::Add,
                    "remove" => ProjectRelatedChannelOperation::Remove,
                    _ => {
                        return Err(SdkError::InvalidInput(
                            "op must be `add` or `remove`".into(),
                        ))
                    }
                };
                if operation.replace(parsed).is_some() {
                    return Err(SdkError::InvalidInput("duplicate `op` tag".into()));
                }
            }
            // Kind 47010 is a regular event: `d` is deliberately the
            // filterable channel coordinate, not a NIP-33 replacement key.
            "d" => {
                let raw = require_pair()?;
                let parsed = Uuid::parse_str(raw)
                    .map_err(|_| SdkError::InvalidInput("d target must be a UUID".into()))?;
                if parsed.is_nil() {
                    return Err(SdkError::InvalidInput(
                        "d target must not be the nil UUID".into(),
                    ));
                }
                if raw != parsed.to_string() {
                    return Err(SdkError::InvalidInput(
                        "d target must use canonical lowercase hyphenated UUID form".into(),
                    ));
                }
                if channel_id.replace(parsed).is_some() {
                    return Err(SdkError::InvalidInput("duplicate `d` tag".into()));
                }
            }
            "auth" => {
                if auth_seen || parts.len() != 4 {
                    return Err(SdkError::InvalidInput(
                        "`auth` tag must appear at most once with four elements".into(),
                    ));
                }
                auth_seen = true;
            }
            _ => {
                return Err(SdkError::InvalidInput(format!(
                    "unknown Project related-channel command tag `{name}`"
                )))
            }
        }
    }

    let project_coordinate =
        project_coordinate.ok_or_else(|| SdkError::InvalidInput("missing `a` tag".into()))?;
    let coordinate = ProjectRelatedChannelCoordinate::parse(&project_coordinate)?;
    let operation = operation.ok_or_else(|| SdkError::InvalidInput("missing `op` tag".into()))?;
    let channel_id =
        channel_id.ok_or_else(|| SdkError::InvalidInput("missing target `d` tag".into()))?;
    Ok(ProjectRelatedChannelCommand {
        project_coordinate,
        project_owner: coordinate.owner,
        project_d: coordinate.project_d,
        channel_id,
        operation,
    })
}

/// Build one strict Project related-channel command.
pub fn build_project_related_channel_command(
    project_coordinate: &str,
    channel_id: Uuid,
    operation: ProjectRelatedChannelOperation,
) -> Result<EventBuilder, SdkError> {
    let _ = ProjectRelatedChannelCoordinate::parse(project_coordinate)?;
    if channel_id.is_nil() {
        return Err(SdkError::InvalidInput(
            "d target must not be the nil UUID".into(),
        ));
    }
    let tags = vec![
        Tag::parse(["a", project_coordinate])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
        Tag::parse(["op", operation.as_str()])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
        Tag::parse(["d", &channel_id.to_string()])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
    ];
    Ok(EventBuilder::new(Kind::Custom(KIND_PROJECT_RELATED_CHANNEL as u16), "").tags(tags))
}

/// Build the relay-derived effective related-channel snapshot for one Project.
///
/// Entries are sorted by channel UUID and encoded as `c=[channel]`.
pub fn build_project_related_channels_snapshot(
    project_coordinate: &str,
    project_event_id: &str,
    entries: impl IntoIterator<Item = Uuid>,
    created_at: u64,
) -> Result<EventBuilder, SdkError> {
    let _ = ProjectRelatedChannelCoordinate::parse(project_coordinate)?;
    let project_event_id = canonical_event_id(project_event_id)?;
    let mut entries: Vec<_> = entries.into_iter().collect();
    entries.sort_unstable();
    if entries.len() > PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP {
        return Err(SdkError::InvalidInput(format!(
            "Project related-channel snapshot exceeds {} entries",
            PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP
        )));
    }
    if entries.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SdkError::InvalidInput(
            "Project related-channel snapshot contains duplicate channels".into(),
        ));
    }

    let snapshot_d = buzz_core::project_related_channels::project_related_channels_snapshot_d(
        project_coordinate,
    );
    let mut tags = Vec::with_capacity(entries.len() + 3);
    tags.push(
        Tag::parse(["d", snapshot_d.as_str()])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
    );
    tags.push(
        Tag::parse(["a", project_coordinate])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
    );
    tags.push(
        Tag::parse(["e", project_event_id.as_str()])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
    );
    for channel_id in entries {
        let channel = channel_id.to_string();
        tags.push(
            Tag::parse(["c", channel.as_str()])
                .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
        );
    }
    Ok(EventBuilder::new(
        Kind::Custom(KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT as u16),
        "",
    )
    .tags(tags)
    .custom_created_at(nostr::Timestamp::from(created_at)))
}

/// Parse one relay-derived related-channel snapshot for `project_coordinate`.
///
/// The snapshot envelope is deliberately canonical: one deterministic `d`,
/// one matching `a`, one exact Project-head `e`, then strictly sorted
/// two-element `c` channel tags.
pub fn parse_project_related_channels_snapshot(
    event: &Event,
    project_coordinate: &str,
    project_event_id: &str,
) -> Result<Vec<Uuid>, SdkError> {
    let _ = ProjectRelatedChannelCoordinate::parse(project_coordinate)?;
    let project_event_id = canonical_event_id(project_event_id)?;
    if u32::from(event.kind.as_u16()) != KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT {
        return Err(SdkError::InvalidInput(format!(
            "expected kind {KIND_PROJECT_RELATED_CHANNELS_SNAPSHOT}"
        )));
    }
    if !event.content.is_empty() {
        return Err(SdkError::InvalidInput(
            "Project related-channel snapshot content must be empty".into(),
        ));
    }

    let tags: Vec<&[String]> = event.tags.iter().map(|tag| tag.as_slice()).collect();
    let expected_d = buzz_core::project_related_channels::project_related_channels_snapshot_d(
        project_coordinate,
    );
    if !matches!(tags.first(), Some([name, value]) if name == "d" && value == &expected_d) {
        return Err(SdkError::InvalidInput(
            "Project related-channel snapshot has an invalid d tag".into(),
        ));
    }
    if !matches!(tags.get(1), Some([name, value]) if name == "a" && value == project_coordinate) {
        return Err(SdkError::InvalidInput(
            "Project related-channel snapshot has an invalid a tag".into(),
        ));
    }
    if !matches!(tags.get(2), Some([name, value]) if name == "e" && value == &project_event_id) {
        return Err(SdkError::InvalidInput(
            "Project related-channel snapshot does not match the current Project head".into(),
        ));
    }
    if tags.len().saturating_sub(3) > PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP {
        return Err(SdkError::InvalidInput(format!(
            "Project related-channel snapshot exceeds {} entries",
            PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP
        )));
    }

    let mut channels = Vec::with_capacity(tags.len().saturating_sub(3));
    for tag in tags.into_iter().skip(3) {
        let [name, raw] = tag else {
            return Err(SdkError::InvalidInput(
                "Project related-channel snapshot c tag must have exactly two elements".into(),
            ));
        };
        if name != "c" {
            return Err(SdkError::InvalidInput(format!(
                "unknown Project related-channel snapshot tag `{name}`"
            )));
        }
        let channel = Uuid::parse_str(raw)
            .map_err(|_| SdkError::InvalidInput("snapshot channel must be a UUID".into()))?;
        if channel.is_nil() || raw != &channel.to_string() {
            return Err(SdkError::InvalidInput(
                "snapshot channel must use canonical non-nil UUID form".into(),
            ));
        }
        if channels.last().is_some_and(|previous| previous >= &channel) {
            return Err(SdkError::InvalidInput(
                "snapshot channels must be unique and strictly sorted".into(),
            ));
        }
        channels.push(channel);
    }
    Ok(channels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn coordinate(keys: &Keys) -> String {
        format!("30621:{}:buzz", keys.public_key().to_hex())
    }

    #[test]
    fn round_trips_add_with_ambient_auth() {
        let keys = Keys::generate();
        let auth = Tag::parse(["auth", &"a".repeat(64), &"b".repeat(128), "kind=47010"])
            .expect("valid auth tag shape");
        let event = build_project_related_channel_command(
            &coordinate(&keys),
            Uuid::new_v4(),
            ProjectRelatedChannelOperation::Add,
        )
        .expect("build command")
        .tag(auth)
        .sign_with_keys(&keys)
        .expect("sign command");

        let parsed = parse_project_related_channel_command(&event).expect("parse command");
        assert_eq!(parsed.operation, ProjectRelatedChannelOperation::Add);
    }

    #[test]
    fn remove_request_has_no_cas_tags_and_unknown_tags_are_rejected() {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        assert!(build_project_related_channel_command(
            &coordinate(&keys),
            channel,
            ProjectRelatedChannelOperation::Remove,
        )
        .is_ok());

        let event = EventBuilder::new(Kind::Custom(KIND_PROJECT_RELATED_CHANNEL as u16), "")
            .tags([
                Tag::parse(["a", &coordinate(&keys)]).expect("a"),
                Tag::parse(["op", "add"]).expect("op"),
                Tag::parse(["d", &channel.to_string()]).expect("channel"),
                Tag::parse(["future", "value"]).expect("future"),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(parse_project_related_channel_command(&event).is_err());
    }

    #[test]
    fn parser_rejects_noncanonical_channel_uuid_spelling() {
        let keys = Keys::generate();
        let channel = Uuid::new_v4().to_string().to_uppercase();
        let event = EventBuilder::new(Kind::Custom(KIND_PROJECT_RELATED_CHANNEL as u16), "")
            .tags([
                Tag::parse(["a", &coordinate(&keys)]).expect("a"),
                Tag::parse(["op", "add"]).expect("op"),
                Tag::parse(["d", &channel]).expect("channel"),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(parse_project_related_channel_command(&event).is_err());
    }

    #[test]
    fn snapshot_builder_sorts_channels_and_derives_coordinate() {
        let keys = Keys::generate();
        let project_coordinate = coordinate(&keys);
        let project_event_id = "11".repeat(32);
        let channel_a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("channel a");
        let channel_b = Uuid::parse_str("00000000-0000-0000-0000-000000000002").expect("channel b");
        let event = build_project_related_channels_snapshot(
            &project_coordinate,
            &project_event_id,
            [channel_b, channel_a],
            123,
        )
        .expect("build snapshot")
        .sign_with_keys(&keys)
        .expect("sign snapshot");
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert_eq!(tags[0][0], "d");
        assert_eq!(
            tags[0][1],
            buzz_core::project_related_channels::project_related_channels_snapshot_d(
                &project_coordinate
            )
        );
        assert_eq!(tags[1][0], "a");
        assert_eq!(tags[1][1], project_coordinate);
        assert_eq!(tags[2], vec!["e".to_owned(), project_event_id.clone()]);
        assert_eq!(tags[3], vec!["c".to_owned(), channel_a.to_string()]);
        assert_eq!(tags[4], vec!["c".to_owned(), channel_b.to_string()]);
        assert_eq!(
            parse_project_related_channels_snapshot(
                &event,
                &project_coordinate,
                &project_event_id,
            )
                .expect("parse snapshot"),
            vec![channel_a, channel_b]
        );
        assert!(parse_project_related_channels_snapshot(
            &event,
            &project_coordinate,
            &"22".repeat(32),
        )
        .is_err());
        assert!(build_project_related_channels_snapshot(
            &project_coordinate,
            &"AA".repeat(32),
            [],
            123,
        )
        .is_err());
    }
}
