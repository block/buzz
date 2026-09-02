//! NIP-PC collaborative Project related-channel changes.

use std::collections::BTreeSet;
use std::sync::Arc;

use buzz_core::event::StoredEvent;
use buzz_core::tenant::TenantContext;
use buzz_db::project_state::{ProjectChangeApplyResult, ProjectRelatedChannelChange};
use nostr::Event;
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;

use super::event::dispatch_persistent_event;
use super::ingest::{IngestError, IngestResult};

const MAX_PATCH_CHANNELS: usize = 64;

#[derive(Debug)]
struct ParsedChange {
    owner: Vec<u8>,
    d_tag: String,
    expected_revision: i64,
    add: Vec<Uuid>,
    remove: Vec<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeContent {
    v: u8,
    patch: ChangePatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangePatch {
    related_channels: RelatedChannelsPatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelatedChannelsPatch {
    add: Vec<String>,
    remove: Vec<String>,
}

fn invalid(message: impl Into<String>) -> IngestError {
    IngestError::Rejected(format!("invalid: {}", message.into()))
}

fn parse_revision(value: &str) -> Result<i64, IngestError> {
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid(
            "expected-revision must be a canonical positive integer",
        ));
    }
    value
        .parse::<i64>()
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or_else(|| invalid("expected-revision is out of range"))
}

fn parse_coordinate(value: &str) -> Result<(Vec<u8>, String), IngestError> {
    let mut parts = value.splitn(3, ':');
    let (Some("30621"), Some(owner_hex), Some(d_tag)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid("a tag must contain a canonical Project coordinate"));
    };
    if owner_hex.len() != 64
        || !owner_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "Project owner must be 64 lowercase hexadecimal characters",
        ));
    }
    if d_tag.is_empty() || d_tag.len() > buzz_db::event::D_TAG_MAX_LEN {
        return Err(invalid("Project d tag is empty or too long"));
    }
    let owner = hex::decode(owner_hex).map_err(|_| invalid("invalid Project owner"))?;
    Ok((owner, d_tag.to_owned()))
}

fn parse_channels(values: Vec<String>) -> Result<Vec<Uuid>, IngestError> {
    if values.len() > MAX_PATCH_CHANNELS {
        return Err(invalid("related-channel patch exceeds 64 entries"));
    }
    values
        .into_iter()
        .map(|value| {
            Uuid::parse_str(&value)
                .ok()
                .filter(|channel| channel.to_string() == value)
                .ok_or_else(|| invalid("related channels must be canonical UUIDs"))
        })
        .collect()
}

fn parse(event: &Event) -> Result<ParsedChange, IngestError> {
    if event.tags.len() != 2 {
        return Err(invalid(
            "Project change must contain exactly an a tag and expected-revision tag",
        ));
    }
    let mut coordinate = None;
    let mut revision = None;
    for tag in event.tags.iter() {
        match tag.as_slice() {
            [name, value] if name == "a" && coordinate.is_none() => {
                coordinate = Some(parse_coordinate(value)?);
            }
            [name, value] if name == "expected-revision" && revision.is_none() => {
                revision = Some(parse_revision(value)?);
            }
            _ => return Err(invalid("Project change tags are malformed")),
        }
    }
    let (owner, d_tag) = coordinate.ok_or_else(|| invalid("missing a tag"))?;
    let expected_revision = revision.ok_or_else(|| invalid("missing expected-revision tag"))?;
    let content: ChangeContent =
        serde_json::from_str(&event.content).map_err(|error| invalid(error.to_string()))?;
    if content.v != 1 {
        return Err(invalid("unsupported Project change version"));
    }
    let add = parse_channels(content.patch.related_channels.add)?;
    let remove = parse_channels(content.patch.related_channels.remove)?;
    if add.is_empty() && remove.is_empty() {
        return Err(invalid("Project change must not be empty"));
    }
    let add_set = add.iter().copied().collect::<BTreeSet<_>>();
    let remove_set = remove.iter().copied().collect::<BTreeSet<_>>();
    if add_set.len() != add.len() || remove_set.len() != remove.len() {
        return Err(invalid("Project change contains a duplicate channel"));
    }
    if !add_set.is_disjoint(&remove_set) {
        return Err(invalid("Project change adds and removes the same channel"));
    }
    Ok(ParsedChange {
        owner,
        d_tag,
        expected_revision,
        add,
        remove,
    })
}

pub(crate) async fn handle(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<IngestResult, IngestError> {
    let parsed = parse(event)?;
    let outcome = state
        .db
        .apply_project_related_channel_change(
            tenant.community(),
            event,
            ProjectRelatedChannelChange {
                project_owner: &parsed.owner,
                project_d_tag: &parsed.d_tag,
                expected_revision: parsed.expected_revision,
                add: &parsed.add,
                remove: &parsed.remove,
            },
        )
        .await
        .map_err(|error| IngestError::Internal(format!("error: apply Project change: {error}")))?;

    let message = match outcome {
        ProjectChangeApplyResult::Applied { revision } => {
            dispatch_persistent_event(
                tenant,
                state,
                &StoredEvent::new(event.clone(), None),
                buzz_core::kind::KIND_PROJECT_CHANGE,
                &event.pubkey.to_hex(),
                None,
            )
            .await;
            format!("revision: {revision}")
        }
        ProjectChangeApplyResult::Duplicate { applied_revision } => {
            format!("duplicate: already applied at revision {applied_revision}")
        }
        ProjectChangeApplyResult::ProjectNotFound => return Err(invalid("Project not found")),
        ProjectChangeApplyResult::ProjectDeleted => {
            return Err(IngestError::Rejected("conflict: Project is deleted".into()))
        }
        ProjectChangeApplyResult::Forbidden => {
            return Err(IngestError::Rejected(
                "restricted: actor cannot manage this Project".into(),
            ))
        }
        ProjectChangeApplyResult::Conflict { current_revision } => {
            return Err(IngestError::Rejected(format!(
                "conflict: Project revision is {current_revision}"
            )))
        }
        ProjectChangeApplyResult::Invalid(message) => return Err(invalid(message)),
    };

    if let Err(error) = super::project_state_projection::publish_project_state_for_coordinate(
        tenant,
        state,
        &parsed.owner,
        &parsed.d_tag,
    )
    .await
    {
        tracing::warn!(
            project_owner = %hex::encode(&parsed.owner),
            project_d_tag = %parsed.d_tag,
            %error,
            "accepted Project change awaits projection repair"
        );
    }
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message,
    })
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::*;

    fn command(tags: Vec<Tag>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(47_010), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign test command")
    }

    fn valid_tags() -> Vec<Tag> {
        vec![
            Tag::parse(["a", &format!("30621:{}:project:x", "a".repeat(64))])
                .expect("coordinate tag"),
            Tag::parse(["expected-revision", "1"]).expect("revision tag"),
        ]
    }

    fn valid_content() -> &'static str {
        r#"{"v":1,"patch":{"related_channels":{"add":["11111111-1111-4111-8111-111111111111"],"remove":[]}}}"#
    }

    #[test]
    fn parses_only_the_exact_v1_envelope() {
        let parsed = parse(&command(valid_tags(), valid_content())).expect("valid command");
        assert_eq!(parsed.expected_revision, 1);
        assert_eq!(parsed.d_tag, "project:x");
        assert_eq!(parsed.add.len(), 1);

        let mut extra = valid_tags();
        extra.push(
            Tag::parse(["auth", &"a".repeat(64), "kind=47010", &"b".repeat(128)])
                .expect("auth tag"),
        );
        assert!(parse(&command(extra, valid_content())).is_err());
        let mut bad_revision = valid_tags();
        bad_revision[1] = Tag::parse(["expected-revision", "01"]).expect("tag");
        assert!(parse(&command(bad_revision, valid_content())).is_err());
        assert!(parse(&command(
            valid_tags(),
            r#"{"v":1,"patch":{"related_channels":{"add":[],"remove":[],"future":true}}}"#,
        ))
        .is_err());
    }
}
