use buzz_core::kind::KIND_GROUP_STATE;
use nostr::Event;
use serde::Serialize;
use uuid::Uuid;

use crate::client::{create_response_with_id, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::parse_uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct GroupSnapshot {
    pub group_id: String,
    pub handle: String,
    pub name: String,
    pub description: Option<String>,
    pub creator: String,
    pub members: Vec<String>,
    pub default_channels: Vec<String>,
    pub created_at: u64,
}

fn tag_values(event: &serde_json::Value, key: &str) -> Vec<String> {
    event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| {
            let values = tag.as_array()?;
            if values.first()?.as_str()? != key {
                return None;
            }
            values.get(1)?.as_str().map(str::to_string)
        })
        .collect()
}

fn has_tag(event: &serde_json::Value, key: &str) -> bool {
    event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_array()
                    .and_then(|values| values.first())
                    .and_then(serde_json::Value::as_str)
                    == Some(key)
            })
        })
}

pub(crate) fn parse_group_snapshot(event: &serde_json::Value) -> Option<GroupSnapshot> {
    if event.get("kind").and_then(serde_json::Value::as_u64) != Some(KIND_GROUP_STATE as u64)
        || has_tag(event, "deleted")
    {
        return None;
    }

    let mut members = tag_values(event, "p");
    members.sort();
    members.dedup();
    let mut default_channels = tag_values(event, "channel");
    default_channels.sort();
    default_channels.dedup();
    let description = tag_values(event, "description")
        .into_iter()
        .next()
        .filter(|value| !value.is_empty());

    Some(GroupSnapshot {
        group_id: tag_values(event, "d").into_iter().next()?,
        handle: tag_values(event, "handle").into_iter().next()?,
        name: tag_values(event, "name").into_iter().next()?,
        description,
        creator: tag_values(event, "creator").into_iter().next()?,
        members,
        default_channels,
        created_at: event
            .get("created_at")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    })
}

pub(crate) async fn fetch_active_groups(
    client: &BuzzClient,
) -> Result<Vec<GroupSnapshot>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_GROUP_STATE],
    });
    let events = client.query_all(filter).await?;
    let mut groups: Vec<GroupSnapshot> = events.iter().filter_map(parse_group_snapshot).collect();
    groups.sort_by(|left, right| {
        left.handle
            .cmp(&right.handle)
            .then_with(|| left.group_id.cmp(&right.group_id))
    });
    Ok(groups)
}

pub(crate) async fn resolve_group(
    client: &BuzzClient,
    reference: &str,
) -> Result<GroupSnapshot, CliError> {
    let group = if let Ok(group_id) = Uuid::parse_str(reference) {
        let group_id = group_id.to_string();
        let filter = serde_json::json!({
            "kinds": [KIND_GROUP_STATE],
            "#d": [group_id],
            "limit": 1,
        });
        let raw = client.query(&filter).await?;
        let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|error| CliError::Other(format!("failed to parse group query: {error}")))?;
        events.first().and_then(parse_group_snapshot)
    } else {
        fetch_active_groups(client)
            .await?
            .into_iter()
            .find(|group| group.handle == reference)
    };

    group.ok_or_else(|| CliError::NotFound(format!("user group '{reference}' not found")))
}

fn format_groups(groups: &[GroupSnapshot], format: &crate::OutputFormat) -> String {
    match format {
        crate::OutputFormat::Json => serde_json::to_string(groups).unwrap_or_default(),
        crate::OutputFormat::Compact => {
            let compact: Vec<serde_json::Value> = groups
                .iter()
                .map(|group| {
                    serde_json::json!({
                        "group_id": group.group_id,
                        "handle": group.handle,
                        "name": group.name,
                        "member_count": group.members.len(),
                    })
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
    }
}

fn validate_write_response(raw: &str) -> Result<String, CliError> {
    let response: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| CliError::Other(format!("relay response is not JSON: {error} ({raw})")))?;
    let accepted = response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let message = response
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if message == "duplicate" || message.starts_with("duplicate:") {
        return Err(CliError::Conflict(message.to_string()));
    }
    if !accepted {
        return Err(CliError::Other(format!("relay rejected event: {message}")));
    }
    Ok(normalize_write_response(raw))
}

fn group_id_from_event(event: &Event) -> Result<String, CliError> {
    event
        .tags
        .iter()
        .find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("g"))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .ok_or_else(|| CliError::Other("group create event is missing its g tag".into()))
}

fn parse_channel_ids(values: &[String]) -> Result<Vec<Uuid>, CliError> {
    values.iter().map(|value| parse_uuid(value)).collect()
}

fn map_group_submit_error(error: CliError) -> CliError {
    match error {
        CliError::Relay { status: 400, body } if body.starts_with("restricted:") => {
            CliError::Auth(body)
        }
        other => other,
    }
}

async fn submit_group_event(client: &BuzzClient, event: Event) -> Result<String, CliError> {
    let raw = client
        .submit_event(event)
        .await
        .map_err(map_group_submit_error)?;
    validate_write_response(&raw)
}

async fn submit_group_builder(
    client: &BuzzClient,
    builder: nostr::EventBuilder,
) -> Result<String, CliError> {
    let event = client.sign_event(builder)?;
    submit_group_event(client, event).await
}

async fn cmd_create(
    client: &BuzzClient,
    handle: &str,
    name: &str,
    description: Option<&str>,
    members: &[String],
    default_channels: &[String],
) -> Result<(), CliError> {
    let channel_ids = parse_channel_ids(default_channels)?;
    let member_refs: Vec<&str> = members.iter().map(String::as_str).collect();
    let builder =
        buzz_sdk::build_group_create(handle, name, description, &member_refs, &channel_ids)
            .map_err(|error| CliError::Usage(format!("invalid group create: {error}")))?;
    let event = client.sign_event(builder)?;
    let group_id = group_id_from_event(&event)?;
    let normalized = submit_group_event(client, event).await?;
    println!(
        "{}",
        create_response_with_id(&normalized, "group_id", &group_id)
    );
    Ok(())
}

async fn cmd_edit(
    client: &BuzzClient,
    reference: &str,
    handle: Option<&str>,
    name: Option<&str>,
    description: Option<&str>,
    default_channels: Option<&[String]>,
) -> Result<(), CliError> {
    let group = resolve_group(client, reference).await?;
    let group_id = parse_uuid(&group.group_id)?;
    let channel_ids = default_channels.map(parse_channel_ids).transpose()?;
    let builder =
        buzz_sdk::build_group_edit(group_id, handle, name, description, channel_ids.as_deref())
            .map_err(|error| CliError::Usage(format!("invalid group edit: {error}")))?;
    println!("{}", submit_group_builder(client, builder).await?);
    Ok(())
}

async fn cmd_delete(client: &BuzzClient, reference: &str) -> Result<(), CliError> {
    let group = resolve_group(client, reference).await?;
    let builder = buzz_sdk::build_group_delete(parse_uuid(&group.group_id)?)
        .map_err(|error| CliError::Usage(format!("invalid group delete: {error}")))?;
    println!("{}", submit_group_builder(client, builder).await?);
    Ok(())
}

async fn cmd_members(
    client: &BuzzClient,
    reference: &str,
    members: &[String],
    add: bool,
) -> Result<(), CliError> {
    let group = resolve_group(client, reference).await?;
    let group_id = parse_uuid(&group.group_id)?;
    let member_refs: Vec<&str> = members.iter().map(String::as_str).collect();
    let builder = if add {
        buzz_sdk::build_group_add_members(group_id, &member_refs)
            .map_err(|error| CliError::Usage(format!("invalid group members: {error}")))?
    } else {
        buzz_sdk::build_group_remove_members(group_id, &member_refs)
            .map_err(|error| CliError::Usage(format!("invalid group members: {error}")))?
    };
    println!("{}", submit_group_builder(client, builder).await?);
    Ok(())
}

pub async fn dispatch(
    cmd: crate::GroupsCmd,
    client: &BuzzClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::GroupsCmd;

    match cmd {
        GroupsCmd::Create {
            handle,
            name,
            description,
            members,
            default_channels,
        } => {
            cmd_create(
                client,
                &handle,
                &name,
                description.as_deref(),
                &members,
                &default_channels,
            )
            .await
        }
        GroupsCmd::Edit {
            group,
            handle,
            name,
            description,
            default_channels,
            clear_default_channels,
        } => {
            let default_channels = if clear_default_channels {
                Some(Vec::new())
            } else if default_channels.is_empty() {
                None
            } else {
                Some(default_channels)
            };
            cmd_edit(
                client,
                &group,
                handle.as_deref(),
                name.as_deref(),
                description.as_deref(),
                default_channels.as_deref(),
            )
            .await
        }
        GroupsCmd::Delete { group } => cmd_delete(client, &group).await,
        GroupsCmd::List => {
            let groups = fetch_active_groups(client).await?;
            println!("{}", format_groups(&groups, format));
            Ok(())
        }
        GroupsCmd::Get { group } => {
            let group = resolve_group(client, &group).await?;
            println!("{}", format_groups(&[group], format));
            Ok(())
        }
        GroupsCmd::AddMembers { group, members } => {
            cmd_members(client, &group, &members, true).await
        }
        GroupsCmd::RemoveMembers { group, members } => {
            cmd_members(client, &group, &members, false).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_groups, map_group_submit_error, parse_group_snapshot, validate_write_response,
        GroupSnapshot,
    };
    use crate::error::CliError;
    use crate::OutputFormat;
    use serde_json::json;

    #[test]
    fn parses_active_snapshot_and_normalizes_lists() {
        let event = json!({
            "kind": 39100,
            "created_at": 42,
            "tags": [
                ["d", "11111111-1111-1111-1111-111111111111"],
                ["handle", "ios-team"],
                ["name", "iOS Team"],
                ["description", "Mobile"],
                ["creator", "creator"],
                ["p", "bbbb"],
                ["p", "aaaa"],
                ["p", "aaaa"],
                ["channel", "22222222-2222-2222-2222-222222222222"]
            ]
        });
        let group = parse_group_snapshot(&event).expect("active group");
        assert_eq!(group.handle, "ios-team");
        assert_eq!(group.members, ["aaaa", "bbbb"]);
        assert_eq!(group.description.as_deref(), Some("Mobile"));
    }

    #[test]
    fn skips_tombstones() {
        let event = json!({
            "kind": 39100,
            "tags": [
                ["d", "11111111-1111-1111-1111-111111111111"],
                ["deleted"]
            ]
        });
        assert!(parse_group_snapshot(&event).is_none());
    }

    #[test]
    fn compact_output_keeps_scriptable_identity_and_count() {
        let group = GroupSnapshot {
            group_id: "id".into(),
            handle: "ios-team".into(),
            name: "iOS Team".into(),
            description: None,
            creator: "creator".into(),
            members: vec!["a".into(), "b".into()],
            default_channels: vec![],
            created_at: 1,
        };
        let output = format_groups(&[group], &OutputFormat::Compact);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).expect("JSON"),
            json!([{
                "group_id": "id",
                "handle": "ios-team",
                "name": "iOS Team",
                "member_count": 2
            }])
        );
    }

    #[test]
    fn duplicate_handle_response_is_a_conflict() {
        let error = validate_write_response(
            r#"{"event_id":"abc","accepted":false,"message":"duplicate: user group handle already exists: ios-team"}"#,
        )
        .expect_err("duplicate handle must fail");
        assert!(matches!(error, CliError::Conflict(_)));
        assert_eq!(crate::error::exit_code(&error), 5);
    }

    #[test]
    fn restricted_group_write_is_an_auth_error() {
        let error = map_group_submit_error(CliError::Relay {
            status: 400,
            body: "restricted: only the group creator may modify this group".into(),
        });
        assert!(matches!(error, CliError::Auth(_)));
        assert_eq!(crate::error::exit_code(&error), 3);
    }
}
