//! Typed, bounded references attached to Board discussion messages.

use nostr::Tag;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Event tag name carrying a Board reference.
pub const BOARD_REFERENCE_TAG: &str = "buzz:board-ref";
/// Current wire version.
pub const BOARD_REFERENCE_VERSION: &str = "1";
/// Maximum references accepted on one message.
pub const MAX_BOARD_REFERENCES: usize = 32;
const MAX_PAYLOAD_BYTES: usize = 4096;
const MAX_LABEL_BYTES: usize = 160;
const MAX_ID_BYTES: usize = 512;

/// As-sent placement used when a live object no longer exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoardPlacement {
    /// Canonical workstream channel UUID.
    pub workstream_id: String,
    /// Least-privilege workstream display label.
    pub workstream_label: String,
    /// Stable Board section identifier.
    pub section_id: String,
    /// As-sent section display label.
    pub section_label: String,
}

/// Immutable semantic snapshot safe to show after live data disappears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoardSnapshot {
    /// Primary display label.
    pub label: String,
    /// Optional secondary state/relationship annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Canonical thread destination bound to a blocker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoardThreadDestination {
    /// Destination channel UUID.
    pub channel_id: String,
    /// Exact target message ID.
    pub message_id: String,
    /// Thread root ID when the target is in a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
}

/// Supported Board domain terms. `kind` is the discriminant on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum BoardReference {
    /// A workstream card.
    Workstream {
        /// Canonical channel UUID.
        identity: String,
        /// As-sent display data.
        snapshot: BoardSnapshot,
        /// As-sent Board placement.
        placement: BoardPlacement,
    },
    /// A Buzz or GitHub pull request.
    PullRequest {
        /// Canonical provider-qualified identity.
        identity: String,
        /// As-sent display data.
        snapshot: BoardSnapshot,
        /// As-sent Board placement.
        placement: BoardPlacement,
    },
    /// An agent identity.
    Agent {
        /// Canonical lowercase hex pubkey.
        identity: String,
        /// As-sent display data.
        snapshot: BoardSnapshot,
        /// As-sent Board placement.
        placement: BoardPlacement,
    },
    /// A stable agent-team identity.
    AgentGroup {
        /// Canonical team identity.
        identity: String,
        /// As-sent display data.
        snapshot: BoardSnapshot,
        /// As-sent Board placement.
        placement: BoardPlacement,
    },
    /// A blocker bound to an exact conversation destination.
    ThreadBlocker {
        /// Stable manual wait key or canonical PR-derived wait identity.
        identity: String,
        /// Exact conversation destination.
        destination: BoardThreadDestination,
        /// As-sent display data.
        snapshot: BoardSnapshot,
        /// As-sent Board placement.
        placement: BoardPlacement,
    },
}

impl BoardReference {
    fn dedup_key(&self) -> (String, &'static str, String) {
        let workstream_id = self.placement().workstream_id.clone();
        match self {
            Self::Workstream { identity, .. } => (workstream_id, "workstream", identity.clone()),
            Self::PullRequest { identity, .. } => (workstream_id, "pull-request", identity.clone()),
            Self::Agent { identity, .. } => (workstream_id, "agent", identity.clone()),
            Self::AgentGroup { identity, .. } => (workstream_id, "agent-group", identity.clone()),
            Self::ThreadBlocker { identity, .. } => {
                (workstream_id, "thread-blocker", identity.clone())
            }
        }
    }

    fn identity(&self) -> &str {
        match self {
            Self::Workstream { identity, .. }
            | Self::PullRequest { identity, .. }
            | Self::Agent { identity, .. }
            | Self::AgentGroup { identity, .. }
            | Self::ThreadBlocker { identity, .. } => identity,
        }
    }

    fn snapshot(&self) -> &BoardSnapshot {
        match self {
            Self::Workstream { snapshot, .. }
            | Self::PullRequest { snapshot, .. }
            | Self::Agent { snapshot, .. }
            | Self::AgentGroup { snapshot, .. }
            | Self::ThreadBlocker { snapshot, .. } => snapshot,
        }
    }

    fn placement(&self) -> &BoardPlacement {
        match self {
            Self::Workstream { placement, .. }
            | Self::PullRequest { placement, .. }
            | Self::Agent { placement, .. }
            | Self::AgentGroup { placement, .. }
            | Self::ThreadBlocker { placement, .. } => placement,
        }
    }

    fn validate(&self) -> bool {
        let placement = self.placement();
        let snapshot = self.snapshot();
        valid_id(self.identity())
            && valid_uuid(&placement.workstream_id)
            && valid_label(&placement.workstream_label)
            && valid_id(&placement.section_id)
            && valid_label(&placement.section_label)
            && valid_label(&snapshot.label)
            && snapshot.detail.as_deref().is_none_or(valid_label)
            && match self {
                Self::Workstream { identity, .. } => identity == &placement.workstream_id,
                Self::Agent { identity, .. } => valid_hex64(identity),
                Self::ThreadBlocker { destination, .. } => {
                    valid_uuid(&destination.channel_id)
                        && valid_hex64(&destination.message_id)
                        && destination
                            .thread_root_id
                            .as_deref()
                            .is_none_or(valid_hex64)
                }
                _ => true,
            }
    }
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_LABEL_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_ID_BYTES && !value.chars().any(char::is_control)
}

fn valid_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok() && value == value.to_ascii_lowercase()
}

/// Build validated ordered event tags. Invalid input rejects the whole send.
pub fn build_board_reference_tags(
    references: &[BoardReference],
) -> Result<Vec<Tag>, crate::SdkError> {
    if references.len() > MAX_BOARD_REFERENCES {
        return Err(crate::SdkError::InvalidInput(format!(
            "too many Board references (max {MAX_BOARD_REFERENCES})"
        )));
    }
    let mut identities = HashSet::new();
    references
        .iter()
        .map(|reference| {
            if !reference.validate() || !identities.insert(reference.dedup_key()) {
                return Err(crate::SdkError::InvalidInput(
                    "invalid or duplicate Board reference".into(),
                ));
            }
            let payload = serde_json::to_string(reference)
                .map_err(|error| crate::SdkError::InvalidInput(error.to_string()))?;
            if payload.len() > MAX_PAYLOAD_BYTES {
                return Err(crate::SdkError::InvalidInput(
                    "Board reference payload too large".into(),
                ));
            }
            Tag::parse([
                BOARD_REFERENCE_TAG,
                BOARD_REFERENCE_VERSION,
                payload.as_str(),
            ])
            .map_err(|error| crate::SdkError::InvalidTag(error.to_string()))
        })
        .collect()
}

/// Parse valid references in persisted order. Bad entries fail closed individually.
pub fn parse_board_references(tags: &nostr::Tags) -> Vec<BoardReference> {
    let mut identities = HashSet::new();
    tags.iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            if values.len() != 3
                || values[0] != BOARD_REFERENCE_TAG
                || values[1] != BOARD_REFERENCE_VERSION
                || values[2].len() > MAX_PAYLOAD_BYTES
            {
                return None;
            }
            let reference: BoardReference = serde_json::from_str(&values[2]).ok()?;
            (reference.validate() && identities.insert(reference.dedup_key())).then_some(reference)
        })
        .take(MAX_BOARD_REFERENCES)
        .collect()
}

/// Parse references authorized for one current Board workstream.
pub fn parse_board_references_for_workstream(
    tags: &nostr::Tags,
    workstream_id: &str,
) -> Vec<BoardReference> {
    parse_board_references(tags)
        .into_iter()
        .filter(|reference| reference.placement().workstream_id == workstream_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Tags;

    fn reference(workstream_id: &str) -> BoardReference {
        BoardReference::Workstream {
            identity: workstream_id.into(),
            snapshot: BoardSnapshot {
                label: "Checkout recovery".into(),
                detail: Some("Active".into()),
            },
            placement: BoardPlacement {
                workstream_id: workstream_id.into(),
                workstream_label: "checkout-recovery".into(),
                section_id: "workstream".into(),
                section_label: "Workstream".into(),
            },
        }
    }

    #[test]
    fn round_trip_preserves_order() {
        let tags = build_board_reference_tags(&[
            reference("123e4567-e89b-12d3-a456-426614174000"),
            reference("123e4567-e89b-12d3-a456-426614174001"),
        ])
        .unwrap();
        let parsed = parse_board_references(&Tags::from_list(tags));
        assert_eq!(
            parsed,
            vec![
                reference("123e4567-e89b-12d3-a456-426614174000"),
                reference("123e4567-e89b-12d3-a456-426614174001")
            ]
        );
    }

    #[test]
    fn malformed_unknown_and_duplicate_entries_fail_closed() {
        let mut tags =
            build_board_reference_tags(&[reference("123e4567-e89b-12d3-a456-426614174000")])
                .unwrap();
        tags.push(Tag::parse([BOARD_REFERENCE_TAG, "99", "{}"]).unwrap());
        tags.push(Tag::parse([BOARD_REFERENCE_TAG, BOARD_REFERENCE_VERSION, "not-json"]).unwrap());
        tags.extend(
            build_board_reference_tags(&[
                reference("123e4567-e89b-12d3-a456-426614174000"),
                reference("123e4567-e89b-12d3-a456-426614174001"),
            ])
            .unwrap(),
        );
        assert_eq!(
            parse_board_references(&Tags::from_list(tags)),
            vec![
                reference("123e4567-e89b-12d3-a456-426614174000"),
                reference("123e4567-e89b-12d3-a456-426614174001")
            ]
        );
    }
    #[test]
    fn strict_unknown_controls_and_forged_workstream_binding_fail_closed() {
        let valid = reference("123e4567-e89b-12d3-a456-426614174000");
        let payload = serde_json::to_value(&valid).unwrap();
        let mut invalid = Vec::new();
        for path in ["top", "snapshot", "placement"] {
            let mut value = payload.clone();
            let target = match path {
                "top" => value.as_object_mut().unwrap(),
                "snapshot" => value.get_mut("snapshot").unwrap().as_object_mut().unwrap(),
                _ => value.get_mut("placement").unwrap().as_object_mut().unwrap(),
            };
            target.insert("extra".into(), serde_json::json!(true));
            invalid.push(value);
        }
        let mut controlled = payload.clone();
        controlled["snapshot"]["label"] = serde_json::json!("bad\u{0085}label");
        invalid.push(controlled);
        let mut forged = payload;
        forged["identity"] = serde_json::json!("123e4567-e89b-12d3-a456-426614174001");
        invalid.push(forged);

        let mut tags = invalid
            .into_iter()
            .map(|value| {
                Tag::parse([
                    BOARD_REFERENCE_TAG,
                    BOARD_REFERENCE_VERSION,
                    serde_json::to_string(&value).unwrap().as_str(),
                ])
                .unwrap()
            })
            .collect::<Vec<_>>();
        tags.extend(build_board_reference_tags(std::slice::from_ref(&valid)).unwrap());
        assert_eq!(parse_board_references(&Tags::from_list(tags)), vec![valid]);
    }

    #[test]
    fn dedup_key_is_scoped_to_workstream_placement() {
        let first = reference("123e4567-e89b-12d3-a456-426614174000");
        let mut second = first.clone();
        if let BoardReference::Workstream {
            identity,
            placement,
            ..
        } = &mut second
        {
            *identity = "123e4567-e89b-12d3-a456-426614174001".into();
            placement.workstream_id = identity.clone();
        }
        let tags =
            Tags::from_list(build_board_reference_tags(&[first.clone(), second.clone()]).unwrap());
        assert_eq!(parse_board_references(&tags), vec![first, second]);
    }

    #[test]
    fn cross_workstream_references_fail_closed() {
        let allowed = reference("123e4567-e89b-12d3-a456-426614174000");
        let other = reference("123e4567-e89b-12d3-a456-426614174001");
        let tags = Tags::from_list(build_board_reference_tags(&[allowed.clone(), other]).unwrap());
        assert_eq!(
            parse_board_references_for_workstream(&tags, "123e4567-e89b-12d3-a456-426614174000"),
            vec![allowed]
        );
    }
}
