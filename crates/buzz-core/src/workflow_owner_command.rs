//! Managed-workflow owner command wire contract.

use nostr::{Event, EventId, PublicKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kind::{KIND_WORKFLOW_DEF, KIND_WORKFLOW_OWNER_COMMAND};

/// Operation requested by a managed agent owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOwnerOperation {
    /// Propose an agent-signed replacement.
    Update,
    /// Enable executable control state.
    Enable,
    /// Disable executable control state.
    Disable,
    /// Retire executable control state and active listings without claiming to
    /// delete the agent-authored NIP-33 event.
    Retire,
}

/// Signed command content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowOwnerCommandBody {
    /// Requested operation.
    pub operation: WorkflowOwnerOperation,
    /// Proposed YAML, present only for update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaml_definition: Option<String>,
}

/// Parsed command identity and target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowOwnerCommand {
    /// Replay key.
    pub command_id: Uuid,
    /// Agent author of the target coordinate.
    pub agent_pubkey: PublicKey,
    /// Workflow d-tag.
    pub workflow_id: Uuid,
    /// Explicit command recipient; must equal the coordinate author.
    pub recipient: PublicKey,
    /// Exact targeted revision.
    pub expected_revision: EventId,
    /// Requested operation.
    pub operation: WorkflowOwnerOperation,
    /// Proposed update YAML.
    pub yaml_definition: Option<String>,
}

/// Parse and structurally validate a signed command.
pub fn parse_owner_command(event: &Event) -> Result<WorkflowOwnerCommand, String> {
    if event.kind.as_u16() as u32 != KIND_WORKFLOW_OWNER_COMMAND {
        return Err("not a workflow owner command".into());
    }
    event
        .verify()
        .map_err(|error| format!("invalid command signature: {error}"))?;
    let command_id = one_tag(event, "d")?
        .parse()
        .map_err(|_| "invalid command id")?;
    let coordinate = one_tag(event, "a")?;
    let mut parts = coordinate.splitn(3, ':');
    if parts.next().and_then(|v| v.parse::<u32>().ok()) != Some(KIND_WORKFLOW_DEF) {
        return Err("command must target kind:30620".into());
    }
    let agent_pubkey = parts
        .next()
        .ok_or("invalid target coordinate")?
        .parse()
        .map_err(|_| "invalid target agent")?;
    let workflow_id = parts
        .next()
        .ok_or("invalid target coordinate")?
        .parse()
        .map_err(|_| "invalid target workflow")?;
    let expected_revision = one_tag(event, "revision")?
        .parse()
        .map_err(|_| "invalid revision")?;
    let recipient: PublicKey = one_tag(event, "p")?
        .parse()
        .map_err(|_| "invalid command recipient")?;
    if recipient != agent_pubkey {
        return Err("command recipient must equal target agent".into());
    }
    let body: WorkflowOwnerCommandBody = serde_json::from_str(&event.content)
        .map_err(|e| format!("invalid command content: {e}"))?;
    match (&body.operation, &body.yaml_definition) {
        (WorkflowOwnerOperation::Update, Some(yaml)) if !yaml.trim().is_empty() => {}
        (WorkflowOwnerOperation::Update, _) => return Err("update requires yaml_definition".into()),
        (_, None) => {}
        (_, Some(_)) => return Err("only update may include yaml_definition".into()),
    }
    Ok(WorkflowOwnerCommand {
        command_id,
        agent_pubkey,
        workflow_id,
        recipient,
        expected_revision,
        operation: body.operation,
        yaml_definition: body.yaml_definition,
    })
}

fn one_tag<'a>(event: &'a Event, name: &str) -> Result<&'a str, String> {
    let mut values = event.tags.iter().filter_map(|tag| {
        let parts = tag.as_slice();
        (parts.len() >= 2 && parts[0] == name).then_some(parts[1].as_str())
    });
    let value = values.next().ok_or_else(|| format!("missing {name} tag"))?;
    if values.next().is_some() {
        return Err(format!("duplicate {name} tag"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn command(
        owner: &Keys,
        agent: &PublicKey,
        workflow: Uuid,
        revision: EventId,
        command_id: Uuid,
        body: WorkflowOwnerCommandBody,
    ) -> Event {
        EventBuilder::new(
            Kind::Custom(KIND_WORKFLOW_OWNER_COMMAND as u16),
            serde_json::to_string(&body).expect("body"),
        )
        .tags([
            Tag::parse(["d", &command_id.to_string()]).expect("d"),
            Tag::parse([
                "a",
                &format!("{KIND_WORKFLOW_DEF}:{}:{workflow}", agent.to_hex()),
            ])
            .expect("a"),
            Tag::parse(["revision", &revision.to_hex()]).expect("revision"),
            Tag::parse(["p", &agent.to_hex()]).expect("p"),
        ])
        .sign_with_keys(owner)
        .expect("sign")
    }

    #[test]
    fn parses_exact_update_contract() {
        let owner = Keys::generate();
        let agent = Keys::generate().public_key();
        let workflow = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        let revision = EventId::all_zeros();
        let event = command(
            &owner,
            &agent,
            workflow,
            revision,
            command_id,
            WorkflowOwnerCommandBody {
                operation: WorkflowOwnerOperation::Update,
                yaml_definition: Some("name: managed".into()),
            },
        );
        let parsed = parse_owner_command(&event).expect("parse");
        assert_eq!(parsed.command_id, command_id);
        assert_eq!(parsed.agent_pubkey, agent);
        assert_eq!(parsed.workflow_id, workflow);
        assert_eq!(parsed.expected_revision, revision);
    }

    #[test]
    fn rejects_recipient_that_does_not_match_coordinate_author() {
        let owner = Keys::generate();
        let agent = Keys::generate().public_key();
        let mut event = command(
            &owner,
            &agent,
            Uuid::new_v4(),
            EventId::all_zeros(),
            Uuid::new_v4(),
            WorkflowOwnerCommandBody {
                operation: WorkflowOwnerOperation::Enable,
                yaml_definition: None,
            },
        );
        let other = Keys::generate().public_key();
        let tags = event
            .tags
            .iter()
            .map(|tag| {
                if tag.as_slice().first().map(String::as_str) == Some("p") {
                    Tag::parse(["p", &other.to_hex()]).expect("p")
                } else {
                    tag.clone()
                }
            })
            .collect::<Vec<_>>();
        event = EventBuilder::new(event.kind, event.content)
            .tags(tags)
            .sign_with_keys(&owner)
            .expect("sign");
        assert_eq!(
            parse_owner_command(&event).unwrap_err(),
            "command recipient must equal target agent"
        );
    }

    #[test]
    fn rejects_update_without_proposed_definition() {
        let event = command(
            &Keys::generate(),
            &Keys::generate().public_key(),
            Uuid::new_v4(),
            EventId::all_zeros(),
            Uuid::new_v4(),
            WorkflowOwnerCommandBody {
                operation: WorkflowOwnerOperation::Update,
                yaml_definition: None,
            },
        );
        assert_eq!(
            parse_owner_command(&event).unwrap_err(),
            "update requires yaml_definition"
        );
    }

    #[test]
    fn rejects_control_operation_with_update_cargo() {
        let event = command(
            &Keys::generate(),
            &Keys::generate().public_key(),
            Uuid::new_v4(),
            EventId::all_zeros(),
            Uuid::new_v4(),
            WorkflowOwnerCommandBody {
                operation: WorkflowOwnerOperation::Retire,
                yaml_definition: Some("name: forbidden".into()),
            },
        );
        assert_eq!(
            parse_owner_command(&event).unwrap_err(),
            "only update may include yaml_definition"
        );
    }
}
