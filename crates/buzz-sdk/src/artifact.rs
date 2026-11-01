//! NIP-AR channel artifact revisions (kind 45010).
//!
//! The relay owns identity, access and revision order. This module only
//! assembles the envelope tags; `content` and client-defined tags stay opaque.

use buzz_core::{artifact, kind::KIND_ARTIFACT};
use nostr::{EventBuilder, Kind, Tag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::SdkError;

/// Tag names owned by the NIP-AR envelope. Client-defined tags cannot use them.
const ENVELOPE_TAGS: [&str; 8] = ["ar", "d", "h", "type", "title", "op", "root", "prev"];

/// Maximum UTF-8 bytes in an artifact title.
pub const ARTIFACT_TITLE_MAX_BYTES: usize = 512;

/// Lifecycle operation carried by a revision's `op` tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactOp {
    /// First revision of a new identity.
    Create,
    /// Content change that keeps the home channel.
    Update,
    /// Current snapshot published into a new home channel.
    Move,
    /// Soft delete; only `Restore` may follow.
    Delete,
    /// Complete snapshot that revives a deleted artifact.
    Restore,
}

impl ArtifactOp {
    /// The wire value of the `op` tag.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Move => "move",
            Self::Delete => "delete",
            Self::Restore => "restore",
        }
    }
}

/// One complete artifact snapshot to publish.
pub struct ArtifactRevision<'a> {
    /// Stable artifact identity (`d` tag).
    pub artifact_id: Uuid,
    /// Home channel after this revision (`h` tag).
    pub channel_id: Uuid,
    /// Immutable namespaced type.
    pub artifact_type: &'a str,
    /// Lifecycle operation.
    pub op: ArtifactOp,
    /// Event ID of the current accepted revision; required except on create.
    pub prev: Option<&'a str>,
    /// Optional conversation anchor event ID within the home channel.
    pub root: Option<&'a str>,
    /// Display title; required except on delete, where it must be absent.
    pub title: Option<&'a str>,
    /// Client-defined tags, written after the envelope in the given order.
    pub tags: &'a [Vec<String>],
    /// Opaque payload; must be empty on delete.
    pub content: &'a str,
}

fn invalid(message: impl Into<String>) -> SdkError {
    SdkError::InvalidInput(message.into())
}

/// Build a kind 45010 revision, rejecting any envelope the relay would refuse.
///
/// Envelope tags are emitted as `ar`, `d`, `h`, `type`, `title`, `op`, `root`,
/// `prev`, followed by `revision.tags`. A delete carries no title, no content
/// and no client-defined tags.
pub fn build_artifact_revision(revision: &ArtifactRevision) -> Result<EventBuilder, SdkError> {
    artifact::validate_type(revision.artifact_type).map_err(invalid)?;
    if revision.artifact_id.is_nil() || revision.channel_id.is_nil() {
        return Err(invalid("artifact and channel IDs must be non-nil UUIDs"));
    }
    if (revision.op == ArtifactOp::Create) != revision.prev.is_none() {
        return Err(invalid("prev is required on every revision except create"));
    }
    for (name, value) in [("prev", revision.prev), ("root", revision.root)] {
        if let Some(value) = value {
            artifact::event_id(value).map_err(|e| invalid(format!("{name}: {e}")))?;
        }
    }
    if revision.op == ArtifactOp::Delete {
        if revision.title.is_some() || !revision.content.is_empty() || !revision.tags.is_empty() {
            return Err(invalid(
                "delete carries no title, content or client-defined tags",
            ));
        }
    } else {
        let title = revision
            .title
            .ok_or_else(|| invalid("title is required unless the operation is delete"))?;
        if title.trim().is_empty() || title.len() > ARTIFACT_TITLE_MAX_BYTES {
            return Err(invalid(format!(
                "title must be nonblank and at most {ARTIFACT_TITLE_MAX_BYTES} UTF-8 bytes"
            )));
        }
    }
    if revision.tags.len() + ENVELOPE_TAGS.len() > artifact::MAX_TAGS {
        return Err(invalid("too many tags"));
    }

    let mut tags = vec![
        Tag::parse(["ar", "1"]),
        Tag::parse(["d", &revision.artifact_id.to_string()]),
        Tag::parse(["h", &revision.channel_id.to_string()]),
        Tag::parse(["type", revision.artifact_type]),
    ];
    if let Some(title) = revision.title {
        tags.push(Tag::parse(["title", title]));
    }
    tags.push(Tag::parse(["op", revision.op.as_str()]));
    if let Some(root) = revision.root {
        tags.push(Tag::parse(["root", root]));
    }
    if let Some(prev) = revision.prev {
        tags.push(Tag::parse(["prev", prev]));
    }
    for extra in revision.tags {
        let name = extra
            .first()
            .ok_or_else(|| invalid("client-defined tags need a name"))?;
        if ENVELOPE_TAGS.contains(&name.as_str()) || name == "auth" {
            return Err(invalid(format!("tag {name:?} is reserved by NIP-AR")));
        }
        tags.push(Tag::parse(extra.iter().map(String::as_str)));
    }
    let tags = tags
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SdkError::InvalidTag(e.to_string()))?;
    Ok(EventBuilder::new(Kind::Custom(KIND_ARTIFACT as u16), revision.content).tags(tags))
}

/// Derive the NIP-AR deterministic artifact identity for `key` within a
/// channel and type: the first 16 bytes of
/// `SHA-256("nip-ar-artifact\0" + h + "\0" + type + "\0" + key)` with the
/// RFC 9562 version 8 and variant bits set. `h` is the canonical lowercase
/// hyphenated channel UUID, as it appears in the `h` tag.
pub fn derive_artifact_id(
    channel_id: Uuid,
    artifact_type: &str,
    key: &str,
) -> Result<Uuid, SdkError> {
    artifact::validate_type(artifact_type).map_err(invalid)?;
    if key.is_empty() {
        return Err(invalid("ensure key must not be empty"));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"nip-ar-artifact\0");
    hasher.update(channel_id.to_string().as_bytes());
    hasher.update([0]);
    hasher.update(artifact_type.as_bytes());
    hasher.update([0]);
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    const D: &str = "04737c81-e5e8-4412-bb47-f446813cfeba";
    const H: &str = "9b353519-f4fe-4757-aef4-bec6cc0ae54c";
    const PREV: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn revision<'a>(op: ArtifactOp, tags: &'a [Vec<String>]) -> ArtifactRevision<'a> {
        ArtifactRevision {
            artifact_id: Uuid::parse_str(D).unwrap(),
            channel_id: Uuid::parse_str(H).unwrap(),
            artifact_type: "buzz.task",
            op,
            prev: (op != ArtifactOp::Create).then_some(PREV),
            root: None,
            title: (op != ArtifactOp::Delete).then_some("Fix the payment timeout"),
            tags,
            content: if op == ArtifactOp::Delete { "" } else { "{}" },
        }
    }

    fn tags_of(revision: &ArtifactRevision) -> Vec<Vec<String>> {
        let event = build_artifact_revision(revision)
            .unwrap()
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(
            artifact::validate(&event).is_ok(),
            "relay validation must accept the built event"
        );
        event.tags.iter().map(|t| t.as_slice().to_vec()).collect()
    }

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tag_shape_per_operation_matches_the_nip() {
        let extra = [strings(&["project", "p1"])];
        let base = |title: bool, op: &str, prev: bool| {
            let mut tags = vec![
                strings(&["ar", "1"]),
                strings(&["d", D]),
                strings(&["h", H]),
                strings(&["type", "buzz.task"]),
            ];
            if title {
                tags.push(strings(&["title", "Fix the payment timeout"]));
            }
            tags.push(strings(&["op", op]));
            if prev {
                tags.push(strings(&["prev", PREV]));
            }
            tags
        };
        let mut create = base(true, "create", false);
        create.push(strings(&["project", "p1"]));
        assert_eq!(tags_of(&revision(ArtifactOp::Create, &extra)), create);
        for (op, name) in [
            (ArtifactOp::Update, "update"),
            (ArtifactOp::Move, "move"),
            (ArtifactOp::Restore, "restore"),
        ] {
            assert_eq!(
                tags_of(&revision(op, &[])),
                base(true, name, true),
                "{name}"
            );
        }
        assert_eq!(
            tags_of(&revision(ArtifactOp::Delete, &[])),
            base(false, "delete", true)
        );
    }

    #[test]
    fn root_is_placed_before_prev() {
        let mut rev = revision(ArtifactOp::Update, &[]);
        rev.root = Some(PREV);
        let tags = tags_of(&rev);
        let names: Vec<&str> = tags.iter().map(|t| t[0].as_str()).collect();
        assert_eq!(
            names,
            ["ar", "d", "h", "type", "title", "op", "root", "prev"]
        );
    }

    #[test]
    fn invalid_revisions_are_rejected_before_signing() {
        let reserved = [strings(&["title", "again"])];
        let auth = [strings(&["auth", "x"])];
        let custom = [strings(&["project", "p1"])];
        let long_title = "x".repeat(ARTIFACT_TITLE_MAX_BYTES + 1);
        let cases: Vec<(&str, ArtifactRevision)> = vec![
            ("create with prev", {
                let mut r = revision(ArtifactOp::Create, &[]);
                r.prev = Some(PREV);
                r
            }),
            ("update without prev", {
                let mut r = revision(ArtifactOp::Update, &[]);
                r.prev = None;
                r
            }),
            ("short prev", {
                let mut r = revision(ArtifactOp::Update, &[]);
                r.prev = Some(&PREV[..63]);
                r
            }),
            ("bad root", {
                let mut r = revision(ArtifactOp::Update, &[]);
                r.root = Some("nope");
                r
            }),
            ("undotted type", {
                let mut r = revision(ArtifactOp::Create, &[]);
                r.artifact_type = "task";
                r
            }),
            ("uppercase type", {
                let mut r = revision(ArtifactOp::Create, &[]);
                r.artifact_type = "Buzz.task";
                r
            }),
            ("blank title", {
                let mut r = revision(ArtifactOp::Create, &[]);
                r.title = Some("  ");
                r
            }),
            ("missing title", {
                let mut r = revision(ArtifactOp::Create, &[]);
                r.title = None;
                r
            }),
            ("long title", {
                let mut r = revision(ArtifactOp::Create, &[]);
                r.title = Some(&long_title);
                r
            }),
            ("delete with title", {
                let mut r = revision(ArtifactOp::Delete, &[]);
                r.title = Some("t");
                r
            }),
            ("delete with content", {
                let mut r = revision(ArtifactOp::Delete, &[]);
                r.content = "x";
                r
            }),
            ("delete with extra tags", {
                revision(ArtifactOp::Delete, &custom)
            }),
            (
                "envelope tag as client tag",
                revision(ArtifactOp::Create, &reserved),
            ),
            (
                "auth tag as client tag",
                revision(ArtifactOp::Create, &auth),
            ),
            ("nil artifact", {
                let mut r = revision(ArtifactOp::Create, &[]);
                r.artifact_id = Uuid::nil();
                r
            }),
        ];
        for (name, rev) in cases {
            assert!(build_artifact_revision(&rev).is_err(), "{name}");
        }
    }

    #[test]
    fn derived_identity_is_stable_versioned_and_scoped() {
        let channel = Uuid::parse_str(H).unwrap();
        let id = derive_artifact_id(channel, "buzz.config", "theme").unwrap();
        assert_eq!(id.to_string(), "e1918051-bdaa-8f23-8b3f-512f1fdf4129");
        assert_eq!(id.get_version_num(), 8);
        assert_eq!(id.get_variant(), uuid::Variant::RFC4122);
        assert_eq!(artifact::canonical_uuid(&id.to_string()), Ok(id));
        for n in 0..64 {
            let derived = derive_artifact_id(channel, "buzz.config", &format!("key-{n}")).unwrap();
            assert_eq!(derived.get_version_num(), 8, "key-{n}");
            assert_eq!(derived.get_variant(), uuid::Variant::RFC4122, "key-{n}");
        }
        assert_eq!(
            derive_artifact_id(channel, "buzz.config", "theme").unwrap(),
            id
        );
        for other in [
            derive_artifact_id(channel, "buzz.config", "other").unwrap(),
            derive_artifact_id(channel, "buzz.other", "theme").unwrap(),
            derive_artifact_id(Uuid::parse_str(D).unwrap(), "buzz.config", "theme").unwrap(),
        ] {
            assert_ne!(other, id);
        }
        assert!(derive_artifact_id(channel, "buzz.config", "").is_err());
        assert!(derive_artifact_id(channel, "config", "theme").is_err());
    }
}
