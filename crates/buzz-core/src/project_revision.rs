//! NIP-MP collaborative Project revision primitives.

use nostr::Event;
use thiserror::Error;
use uuid::Uuid;

use crate::kind::{KIND_PROJECT, KIND_PROJECT_REVISION};

/// Mutation encoded by a [`KIND_PROJECT_REVISION`] event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectRevisionOperation {
    /// Add one extra channel to the Project.
    AddRelatedChannel,
    /// Remove one extra channel from the Project.
    RemoveRelatedChannel,
}

impl ProjectRevisionOperation {
    /// Canonical wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AddRelatedChannel => "add-related-channel",
            Self::RemoveRelatedChannel => "remove-related-channel",
        }
    }

    fn parse(value: &str) -> Result<Self, ProjectRevisionError> {
        match value {
            "add-related-channel" => Ok(Self::AddRelatedChannel),
            "remove-related-channel" => Ok(Self::RemoveRelatedChannel),
            _ => Err(ProjectRevisionError::InvalidOperation),
        }
    }
}

/// A parsed Project coordinate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectCoordinate {
    /// Project owner/signer as lowercase hex.
    pub owner: String,
    /// Project `d` tag, preserved verbatim.
    pub slug: String,
}

impl ProjectCoordinate {
    /// Parse the exact `30621:<owner-hex>:<d>` form.
    pub fn parse(value: &str) -> Result<Self, ProjectRevisionError> {
        let mut parts = value.splitn(3, ':');
        if parts.next() != Some("30621") {
            return Err(ProjectRevisionError::InvalidProjectCoordinate);
        }
        let owner = parts
            .next()
            .filter(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
            .ok_or(ProjectRevisionError::InvalidProjectCoordinate)?;
        let slug = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or(ProjectRevisionError::InvalidProjectCoordinate)?;
        Ok(Self {
            owner: owner.to_owned(),
            slug: slug.to_owned(),
        })
    }

    /// Return the canonical coordinate.
    #[must_use]
    pub fn as_string(&self) -> String {
        format!("{KIND_PROJECT}:{}:{}", self.owner, self.slug)
    }
}

/// Validated contents of a Project revision event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRevision {
    /// Stable Project coordinate.
    pub project: ProjectCoordinate,
    /// Current owner-signed Project event on which this revision chain is based.
    pub base_revision: String,
    /// Exact base or prior revision event id.
    pub expected_revision: String,
    /// Requested mutation.
    pub operation: ProjectRevisionOperation,
    /// Related channel being changed.
    pub channel_id: Uuid,
    /// Complete related-channel state after applying this revision.
    pub related_channels: Vec<Uuid>,
}

/// Project revision envelope error.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProjectRevisionError {
    /// Wrong event kind.
    #[error("event is not a Project revision")]
    WrongKind,
    /// A required singleton tag was missing or repeated.
    #[error("Project revision requires exactly one {0} tag")]
    TagCardinality(&'static str),
    /// Project coordinate is malformed.
    #[error("invalid Project coordinate")]
    InvalidProjectCoordinate,
    /// Revision event id is malformed.
    #[error("invalid expected Project revision")]
    InvalidExpectedRevision,
    /// Base Project event id is malformed.
    #[error("invalid base Project revision")]
    InvalidBaseRevision,
    /// Operation is unsupported.
    #[error("invalid Project revision operation")]
    InvalidOperation,
    /// Channel id is malformed.
    #[error("invalid related channel id")]
    InvalidChannel,
    /// Snapshot channel tags are malformed, duplicated, or over the limit.
    #[error("invalid related channel snapshot")]
    InvalidRelatedChannels,
    /// Content must be empty.
    #[error("Project revision content must be empty")]
    NonEmptyContent,
}

fn singleton_tag<'a>(
    event: &'a Event,
    name: &'static str,
) -> Result<&'a str, ProjectRevisionError> {
    let matching: Vec<&[String]> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|parts| parts.first().map(String::as_str) == Some(name))
        .collect();
    match matching.as_slice() {
        [[_, value]] => Ok(value),
        _ => Err(ProjectRevisionError::TagCardinality(name)),
    }
}

impl ProjectRevision {
    /// Parse and validate a signed revision envelope.
    pub fn parse(event: &Event) -> Result<Self, ProjectRevisionError> {
        if u32::from(event.kind.as_u16()) != KIND_PROJECT_REVISION {
            return Err(ProjectRevisionError::WrongKind);
        }
        if !event.content.is_empty() {
            return Err(ProjectRevisionError::NonEmptyContent);
        }
        let project = ProjectCoordinate::parse(singleton_tag(event, "a")?)?;
        let base_revision = singleton_tag(event, "base")?;
        if base_revision.len() != 64 || !base_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ProjectRevisionError::InvalidBaseRevision);
        }
        let expected_revision = singleton_tag(event, "e")?;
        if expected_revision.len() != 64
            || !expected_revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ProjectRevisionError::InvalidExpectedRevision);
        }
        let operation = ProjectRevisionOperation::parse(singleton_tag(event, "op")?)?;
        let channel = singleton_tag(event, "channel")?;
        let channel_id: Uuid = channel
            .parse()
            .map_err(|_| ProjectRevisionError::InvalidChannel)?;
        if channel_id.to_string() != channel {
            return Err(ProjectRevisionError::InvalidChannel);
        }
        let mut related_channels = Vec::new();
        for tag in event.tags.iter() {
            let parts = tag.as_slice();
            if parts.first().map(String::as_str) != Some("buzz-related-channel") {
                continue;
            }
            let [_, value] = parts else {
                return Err(ProjectRevisionError::InvalidRelatedChannels);
            };
            let channel: Uuid = value
                .parse()
                .map_err(|_| ProjectRevisionError::InvalidRelatedChannels)?;
            if channel.to_string() != *value
                || related_channels.contains(&channel)
                || related_channels.len() >= 64
            {
                return Err(ProjectRevisionError::InvalidRelatedChannels);
            }
            related_channels.push(channel);
        }
        Ok(Self {
            project,
            base_revision: base_revision.to_ascii_lowercase(),
            expected_revision: expected_revision.to_ascii_lowercase(),
            operation,
            channel_id,
            related_channels,
        })
    }
}

/// Apply a revision to a set of related channels.
pub fn apply_project_revision(
    channels: &mut Vec<Uuid>,
    home_channel: Option<Uuid>,
    operation: ProjectRevisionOperation,
    channel_id: Uuid,
) -> Result<(), &'static str> {
    if home_channel == Some(channel_id) {
        return Err("the Project home channel cannot also be related");
    }
    match operation {
        ProjectRevisionOperation::AddRelatedChannel => {
            if channels.contains(&channel_id) {
                return Err("channel is already related to the Project");
            }
            if channels.len() >= 64 {
                return Err("Project already has 64 related channels");
            }
            channels.push(channel_id);
        }
        ProjectRevisionOperation::RemoveRelatedChannel => {
            let Some(index) = channels
                .iter()
                .position(|candidate| *candidate == channel_id)
            else {
                return Err("channel is not related to the Project");
            };
            channels.remove(index);
        }
    }
    Ok(())
}

/// Whether an actor may manage a Project under the home-channel rule.
///
/// The Project signer is always authorized. Other actors require an active
/// `owner` or `admin` role in the current, resolvable home channel.
#[must_use]
pub fn can_manage_project(actor: &[u8], project_owner: &[u8], channel_role: Option<&str>) -> bool {
    actor == project_owner || matches!(channel_role, Some("owner" | "admin"))
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::*;

    fn revision_event_with_channel(operation: &str, channel: &str) -> Event {
        let keys = Keys::generate();
        let owner = "a".repeat(64);
        EventBuilder::new(Kind::Custom(KIND_PROJECT_REVISION as u16), "")
            .tags([
                Tag::parse(vec!["a".to_owned(), format!("30621:{owner}:buzz")]).unwrap(),
                Tag::parse(vec!["base".to_owned(), "c".repeat(64)]).unwrap(),
                Tag::parse(vec!["e".to_owned(), "b".repeat(64)]).unwrap(),
                Tag::parse(vec!["op".to_owned(), operation.to_owned()]).unwrap(),
                Tag::parse(vec!["channel".to_owned(), channel.to_owned()]).unwrap(),
                Tag::parse(vec!["buzz-related-channel".to_owned(), channel.to_owned()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn revision_event(operation: &str) -> Event {
        revision_event_with_channel(operation, "11111111-1111-4111-8111-111111111111")
    }

    #[test]
    fn parses_canonical_revision() {
        let parsed = ProjectRevision::parse(&revision_event("add-related-channel")).unwrap();
        assert_eq!(parsed.project.slug, "buzz");
        assert_eq!(parsed.base_revision, "c".repeat(64));
        assert_eq!(
            parsed.operation,
            ProjectRevisionOperation::AddRelatedChannel
        );
        assert_eq!(parsed.related_channels, vec![parsed.channel_id]);
    }

    #[test]
    fn rejects_noncanonical_channel_uuid() {
        assert_eq!(
            ProjectRevision::parse(&revision_event_with_channel(
                "add-related-channel",
                "11111111111141118111111111111111",
            )),
            Err(ProjectRevisionError::InvalidChannel)
        );
    }

    #[test]
    fn rejects_ambiguous_or_unknown_envelopes() {
        let mut duplicate = revision_event("add-related-channel");
        duplicate
            .tags
            .push(Tag::parse(["op", "remove-related-channel"]).unwrap());
        assert_eq!(
            ProjectRevision::parse(&duplicate),
            Err(ProjectRevisionError::TagCardinality("op"))
        );
        assert_eq!(
            ProjectRevision::parse(&revision_event("replace-all")),
            Err(ProjectRevisionError::InvalidOperation)
        );

        let mut malformed_duplicate = revision_event("add-related-channel");
        malformed_duplicate
            .tags
            .push(Tag::parse(["op", "remove-related-channel", "extra"]).unwrap());
        assert_eq!(
            ProjectRevision::parse(&malformed_duplicate),
            Err(ProjectRevisionError::TagCardinality("op"))
        );
    }

    #[test]
    fn rejects_duplicate_or_noncanonical_snapshot_channels() {
        let mut duplicate = revision_event("add-related-channel");
        duplicate.tags.push(
            Tag::parse([
                "buzz-related-channel",
                "11111111-1111-4111-8111-111111111111",
            ])
            .unwrap(),
        );
        assert_eq!(
            ProjectRevision::parse(&duplicate),
            Err(ProjectRevisionError::InvalidRelatedChannels)
        );

        let mut noncanonical = revision_event("add-related-channel");
        noncanonical.tags.pop();
        noncanonical.tags.push(
            Tag::parse(["buzz-related-channel", "11111111111141118111111111111111"]).unwrap(),
        );
        assert_eq!(
            ProjectRevision::parse(&noncanonical),
            Err(ProjectRevisionError::InvalidRelatedChannels)
        );
    }

    #[test]
    fn add_remove_and_home_channel_guards() {
        let home = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let related = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
        let mut channels = Vec::new();
        apply_project_revision(
            &mut channels,
            Some(home),
            ProjectRevisionOperation::AddRelatedChannel,
            related,
        )
        .unwrap();
        assert_eq!(channels, vec![related]);
        assert!(apply_project_revision(
            &mut channels,
            Some(home),
            ProjectRevisionOperation::AddRelatedChannel,
            home,
        )
        .is_err());
        apply_project_revision(
            &mut channels,
            Some(home),
            ProjectRevisionOperation::RemoveRelatedChannel,
            related,
        )
        .unwrap();
        assert!(channels.is_empty());
    }

    #[test]
    fn only_owner_and_home_channel_admin_roles_manage_projects() {
        let owner = [1_u8; 32];
        let actor = [2_u8; 32];
        assert!(can_manage_project(&owner, &owner, None));
        assert!(can_manage_project(&actor, &owner, Some("owner")));
        assert!(can_manage_project(&actor, &owner, Some("admin")));
        for role in [Some("member"), Some("guest"), Some("bot"), None] {
            assert!(!can_manage_project(&actor, &owner, role));
        }
    }
}
