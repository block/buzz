//! Pure canonical serializer for NIP-PC Project State projections.

use nostr::{Event, EventId, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::kind::{KIND_PROJECT, KIND_PROJECT_STATE};

const PROJECT_D_MAX: usize = 1024;

/// Inputs needed to derive a canonical relay-signed Project State event body.
#[derive(Debug, Clone, Copy)]
pub struct ProjectStateProjectionInput<'a> {
    /// Canonical `30621:<owner-hex>:<project-d>` Project coordinate.
    pub coordinate: &'a str,
    /// Monotonic authoritative Project revision.
    pub revision: i64,
    /// Current owner-signed NIP-MP Project identity event.
    pub identity_event: &'a Event,
    /// Identity or deletion event that produced this revision.
    pub change_event_id: &'a EventId,
    /// Whether the Project is currently deleted.
    pub deleted: bool,
    /// Authoritative, duplicate-free related-channel set.
    pub related_channels: &'a [Uuid],
}

/// Unsigned, untimestamped fields for a relay-authored Project State event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectStateTemplate {
    /// Fixed NIP-PC Project State kind (`30623`).
    pub kind: Kind,
    /// Canonically ordered event tags.
    pub tags: Vec<Tag>,
    /// Stable compact JSON projection content.
    pub content: String,
}

/// A structural or encoding failure while deriving or reading Project State.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid Project State: {0}")]
pub struct ProjectStateError(String);

/// Derive the canonical unsigned NIP-PC Project State event template.
///
/// The identity and related-channel set must come from the relay's already
/// validated authoritative state. This function verifies only that the identity
/// kind, signer, and single `d` tag match `coordinate`; it then sorts the
/// related-channel set for stable serialization and enforces the home-channel
/// exclusion.
///
/// Signing and strictly monotonic `created_at` allocation remain relay concerns.
pub fn project_state_template(
    input: ProjectStateProjectionInput<'_>,
) -> Result<ProjectStateTemplate, ProjectStateError> {
    let (owner, project_d) = parse_project_coordinate(input.coordinate)?;
    if input.revision < 1 {
        return Err(ProjectStateError(
            "revision must be a positive signed 64-bit integer".into(),
        ));
    }
    validate_identity_coordinate(input.identity_event, owner, project_d)?;

    let mut related: Vec<String> = input.related_channels.iter().map(Uuid::to_string).collect();
    related.sort_unstable();

    let project_tags = if input.deleted {
        Vec::new()
    } else {
        canonical_live_tags(input.identity_event, project_d, &related)?
    };
    let body = ProjectionBody {
        v: 1,
        deleted: input.deleted,
        project_tags,
    };
    let content = serde_json::to_string(&body)
        .map_err(|error| ProjectStateError(format!("could not encode content: {error}")))?;

    let projection_d = hex::encode(Sha256::digest(input.coordinate.as_bytes()));
    let tags = vec![
        make_tag(vec!["d".into(), projection_d])?,
        make_tag(vec!["a".into(), input.coordinate.into()])?,
        make_tag(vec!["rev".into(), input.revision.to_string()])?,
        make_tag(vec![
            "e".into(),
            input.identity_event.id.to_hex(),
            String::new(),
            "identity".into(),
        ])?,
        make_tag(vec![
            "e".into(),
            input.change_event_id.to_hex(),
            String::new(),
            "change".into(),
        ])?,
    ];

    Ok(ProjectStateTemplate {
        kind: Kind::from(KIND_PROJECT_STATE as u16),
        tags,
        content,
    })
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectionBody {
    v: u8,
    deleted: bool,
    project_tags: Vec<Vec<String>>,
}

/// Validate a relay-authored Project State event and return its CAS revision.
///
/// This checks the event signature and advertised relay author, the requested
/// Project coordinate, a canonical positive revision, and the strict version-1
/// content shape. The relay remains responsible for constructing canonical
/// effective Project tags.
pub fn validate_project_state_projection(
    event: &Event,
    relay_pubkey: &PublicKey,
    coordinate: &str,
) -> Result<u64, ProjectStateError> {
    parse_project_coordinate(coordinate)?;
    event
        .verify()
        .map_err(|error| ProjectStateError(format!("invalid event signature: {error}")))?;
    if event.kind.as_u16() as u32 != KIND_PROJECT_STATE || &event.pubkey != relay_pubkey {
        return Err(ProjectStateError(
            "event is not a Project State signed by the relay".into(),
        ));
    }

    let coordinate_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(Tag::as_slice)
        .filter(|tag| tag.first().is_some_and(|name| name == "a"))
        .collect();
    if coordinate_tags.as_slice() != [["a", coordinate]] {
        return Err(ProjectStateError(
            "projection must have exactly one matching Project coordinate".into(),
        ));
    }
    let revision_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(Tag::as_slice)
        .filter(|tag| tag.first().is_some_and(|name| name == "rev"))
        .collect();
    let [revision_tag] = revision_tags.as_slice() else {
        return Err(ProjectStateError(
            "projection must have exactly one revision".into(),
        ));
    };
    let [_, revision] = *revision_tag else {
        return Err(ProjectStateError(
            "projection revision tag is malformed".into(),
        ));
    };
    if !canonical_positive_i64(revision) {
        return Err(ProjectStateError(
            "projection revision is not canonical".into(),
        ));
    }
    let revision = revision
        .parse::<u64>()
        .map_err(|error| ProjectStateError(format!("invalid projection revision: {error}")))?;
    let body: ProjectionBody = serde_json::from_str(&event.content)
        .map_err(|error| ProjectStateError(format!("invalid projection JSON: {error}")))?;
    if body.v != 1 {
        return Err(ProjectStateError(
            "projection JSON is not supported version 1".into(),
        ));
    }
    Ok(revision)
}

fn canonical_positive_i64(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<i64>().is_ok_and(|value| value > 0)
}

fn parse_project_coordinate(coordinate: &str) -> Result<(&str, &str), ProjectStateError> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts.next();
    let owner = parts.next();
    let project_d = parts.next();
    if kind != Some("30621") {
        return Err(ProjectStateError("coordinate kind must be 30621".into()));
    }
    let owner = owner.ok_or_else(|| ProjectStateError("coordinate owner is missing".into()))?;
    if owner.len() != 64
        || !owner
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProjectStateError(
            "coordinate owner must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    let project_d = project_d
        .filter(|value| !value.is_empty() && value.len() <= PROJECT_D_MAX)
        .ok_or_else(|| {
            ProjectStateError(format!(
                "coordinate Project d must contain 1..={PROJECT_D_MAX} bytes"
            ))
        })?;
    Ok((owner, project_d))
}

fn validate_identity_coordinate(
    event: &Event,
    owner: &str,
    project_d: &str,
) -> Result<(), ProjectStateError> {
    if event.kind.as_u16() as u32 != KIND_PROJECT {
        return Err(ProjectStateError(
            "identity event kind must be 30621".into(),
        ));
    }
    if event.pubkey.to_hex() != owner {
        return Err(ProjectStateError(
            "identity signer does not match the coordinate owner".into(),
        ));
    }

    let d_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(Tag::as_slice)
        .filter(|parts| parts.first().is_some_and(|name| name == "d"))
        .collect();
    if d_tags.len() != 1 || d_tags[0].get(1).map(String::as_str) != Some(project_d) {
        return Err(ProjectStateError(
            "identity must have exactly one d tag matching the coordinate".into(),
        ));
    }
    Ok(())
}

fn canonical_live_tags(
    identity: &Event,
    project_d: &str,
    related: &[String],
) -> Result<Vec<Vec<String>>, ProjectStateError> {
    let mut name = None;
    let mut description = None;
    let mut members = Vec::new();
    let mut home_channel = None;
    let mut visibility = None;
    let mut extensions = Vec::new();

    for tag in identity.tags.iter() {
        let parts = tag.as_slice().to_vec();
        match parts.first().map(String::as_str) {
            Some("d") => {}
            Some("name") => name = Some(parts),
            Some("description") => description = Some(parts),
            Some("a") => members.push(parts),
            Some("buzz-channel") => home_channel = Some(parts),
            Some("buzz-visibility") => visibility = Some(parts),
            Some("auth" | "buzz-related-channel") => {}
            _ => extensions.push(parts),
        }
    }
    members.sort_unstable();
    if let Some(home) = home_channel
        .as_ref()
        .and_then(|tag| tag.get(1))
        .and_then(|value| Uuid::parse_str(value).ok())
    {
        if related.iter().any(|channel| channel == &home.to_string()) {
            return Err(ProjectStateError(
                "the home channel cannot also be a related channel".into(),
            ));
        }
    }

    let mut tags = vec![vec!["d".into(), project_d.into()]];
    tags.extend(name);
    tags.extend(description);
    tags.extend(members);
    tags.extend(home_channel);
    tags.extend(
        related
            .iter()
            .map(|channel| vec!["buzz-related-channel".into(), channel.clone()]),
    );
    tags.extend(visibility);
    tags.extend(extensions);
    Ok(tags)
}

fn make_tag(parts: Vec<String>) -> Result<Tag, ProjectStateError> {
    Tag::parse(parts).map_err(|error| ProjectStateError(format!("could not encode tag: {error}")))
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Tag};

    use super::*;

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).expect("valid test tag")
    }

    fn fixture(tags: Vec<Tag>) -> (Keys, Event) {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(KIND_PROJECT as u16), "ignored")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign test identity");
        (keys, event)
    }

    fn signed_projection(relay: &Keys, template: &ProjectStateTemplate) -> Event {
        EventBuilder::new(template.kind, &template.content)
            .tags(template.tags.clone())
            .sign_with_keys(relay)
            .expect("sign test projection")
    }

    #[test]
    fn emits_exact_canonical_live_projection() {
        let repo_b = format!("30617:{}:b", "b".repeat(64));
        let repo_a = format!("30617:{}:a", "a".repeat(64));
        let home = "11111111-1111-4111-8111-111111111111";
        let related_a = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
        let related_b = Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap();
        let (keys, identity) = fixture(vec![
            tag(&["x-ext", "one", "unchanged"]),
            tag(&["a", &repo_b, "wss://b.example"]),
            tag(&["auth", "secret"]),
            tag(&["description", "Desc"]),
            tag(&["d", "project:one"]),
            tag(&[
                "buzz-related-channel",
                "44444444-4444-4444-8444-444444444444",
            ]),
            tag(&["name", "Name"]),
            tag(&["a", &repo_a]),
            tag(&["buzz-visibility", "unlisted"]),
            tag(&["buzz-channel", home]),
            tag(&["z-ext", "two"]),
        ]);
        let change = EventBuilder::new(Kind::TextNote, "change")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let coordinate = format!("30621:{}:project:one", keys.public_key().to_hex());

        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 8,
            identity_event: &identity,
            change_event_id: &change.id,
            deleted: false,
            related_channels: &[related_b, related_a],
        })
        .unwrap();

        let expected_d = hex::encode(Sha256::digest(coordinate.as_bytes()));
        let raw_tags: Vec<Vec<String>> = template
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert_eq!(template.kind.as_u16() as u32, KIND_PROJECT_STATE);
        assert_eq!(
            raw_tags,
            vec![
                vec!["d".into(), expected_d],
                vec!["a".into(), coordinate],
                vec!["rev".into(), "8".into()],
                vec![
                    "e".into(),
                    identity.id.to_hex(),
                    "".into(),
                    "identity".into()
                ],
                vec!["e".into(), change.id.to_hex(), "".into(), "change".into()],
            ]
        );
        assert_eq!(
            template.content,
            format!(
                "{{\"v\":1,\"deleted\":false,\"project_tags\":[[\"d\",\"project:one\"],[\"name\",\"Name\"],[\"description\",\"Desc\"],[\"a\",\"{repo_a}\"],[\"a\",\"{repo_b}\",\"wss://b.example\"],[\"buzz-channel\",\"{home}\"],[\"buzz-related-channel\",\"{related_a}\"],[\"buzz-related-channel\",\"{related_b}\"],[\"buzz-visibility\",\"unlisted\"],[\"x-ext\",\"one\",\"unchanged\"],[\"z-ext\",\"two\"]]}}"
            )
        );
    }

    #[test]
    fn emits_exact_tombstone_content() {
        let (keys, identity) = fixture(vec![tag(&["d", "gone"])]);
        let coordinate = format!("30621:{}:gone", keys.public_key().to_hex());
        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 2,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: true,
            related_channels: &[],
        })
        .unwrap();
        assert_eq!(
            template.content,
            "{\"v\":1,\"deleted\":true,\"project_tags\":[]}"
        );
    }

    #[test]
    fn hashes_max_length_colon_bearing_project_d() {
        let project_d = format!("prefix:{}", "x".repeat(PROJECT_D_MAX - 7));
        assert_eq!(project_d.len(), PROJECT_D_MAX);
        let (keys, identity) = fixture(vec![tag(&["d", &project_d])]);
        let coordinate = format!("30621:{}:{project_d}", keys.public_key().to_hex());

        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 1,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: false,
            related_channels: &[],
        })
        .unwrap();

        assert_eq!(
            template.tags[0].as_slice(),
            [
                "d",
                hex::encode(Sha256::digest(coordinate.as_bytes())).as_str()
            ]
        );
    }

    #[test]
    fn rejects_home_channel_as_related() {
        let home = "11111111-1111-4111-8111-111111111111";
        let (keys, identity) = fixture(vec![tag(&["d", "project"]), tag(&["buzz-channel", home])]);
        let coordinate = format!("30621:{}:project", keys.public_key().to_hex());
        let home = Uuid::parse_str(home).unwrap();

        assert!(project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 1,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: false,
            related_channels: &[home],
        })
        .is_err());
    }

    #[test]
    fn rejects_mismatched_identity() {
        let (keys, identity) = fixture(vec![tag(&["d", "project"])]);
        let wrong_coordinate = format!("30621:{}:other", keys.public_key().to_hex());

        assert!(project_state_template(ProjectStateProjectionInput {
            coordinate: &wrong_coordinate,
            revision: 1,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: false,
            related_channels: &[],
        })
        .is_err());
    }

    #[test]
    fn validates_relay_coordinate_revision_and_strict_v1_body() {
        let (owner, identity) = fixture(vec![tag(&["d", "project"])]);
        let relay = Keys::generate();
        let coordinate = format!("30621:{}:project", owner.public_key().to_hex());
        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 7,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: false,
            related_channels: &[],
        })
        .unwrap();
        let event = signed_projection(&relay, &template);
        assert_eq!(
            validate_project_state_projection(&event, &relay.public_key(), &coordinate),
            Ok(7)
        );

        let mut reordered = template.clone();
        reordered.tags.swap(0, 1);
        assert_eq!(
            validate_project_state_projection(
                &signed_projection(&relay, &reordered),
                &relay.public_key(),
                &coordinate,
            ),
            Ok(7)
        );

        let impostor = Keys::generate();
        assert!(validate_project_state_projection(
            &signed_projection(&impostor, &template),
            &relay.public_key(),
            &coordinate,
        )
        .is_err());
        assert!(validate_project_state_projection(
            &event,
            &relay.public_key(),
            &format!("30621:{}:other", owner.public_key().to_hex()),
        )
        .is_err());

        let mut noncanonical_revision = template.clone();
        noncanonical_revision.tags[2] = tag(&["rev", "07"]);
        assert!(validate_project_state_projection(
            &signed_projection(&relay, &noncanonical_revision),
            &relay.public_key(),
            &coordinate,
        )
        .is_err());

        let mut unknown_field = template;
        unknown_field.content = r#"{"v":1,"deleted":false,"project_tags":[],"future":true}"#.into();
        assert!(validate_project_state_projection(
            &signed_projection(&relay, &unknown_field),
            &relay.public_key(),
            &coordinate,
        )
        .is_err());
    }
}
