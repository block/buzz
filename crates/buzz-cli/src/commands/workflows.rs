use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};

use crate::client::{
    extract_d_tag, extract_relay_response_field, normalize_write_response, print_create_response,
    BuzzClient,
};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, sdk_err, validate_uuid};

// TODO(phase-4): Replace raw nostr::EventBuilder usage with buzz-sdk builder functions

fn json_tag(event: &serde_json::Value, name: &str) -> Option<String> {
    event.get("tags")?.as_array()?.iter().find_map(|tag| {
        let parts = tag.as_array()?;
        (parts.first()?.as_str()? == name)
            .then(|| parts.get(1)?.as_str().map(str::to_string))
            .flatten()
    })
}

fn current_workflow_events(events: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut state_heads: HashMap<String, &serde_json::Value> = HashMap::new();
    for event in events
        .iter()
        .filter(|event| event.get("kind").and_then(|value| value.as_u64()) == Some(30623))
    {
        let Some(workflow_id) = json_tag(event, "d") else {
            continue;
        };
        match state_heads.get(&workflow_id) {
            Some(current) if !json_workflow_state_is_newer(event, current) => {}
            _ => {
                state_heads.insert(workflow_id, event);
            }
        }
    }
    let projected: HashSet<String> = state_heads.keys().cloned().collect();
    events
        .iter()
        .filter(
            |event| match event.get("kind").and_then(|value| value.as_u64()) {
                Some(30623) => json_tag(event, "d").is_some_and(|workflow_id| {
                    state_heads
                        .get(&workflow_id)
                        .is_some_and(|head| head.get("id") == event.get("id"))
                        && json_tag(event, "status").as_deref() != Some("deleted")
                }),
                Some(30620) => json_tag(event, "d").is_some_and(|id| !projected.contains(&id)),
                _ => false,
            },
        )
        .cloned()
        .collect()
}

fn json_workflow_state_is_newer(
    candidate: &serde_json::Value,
    current: &serde_json::Value,
) -> bool {
    let candidate_created = candidate
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let current_created = current
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let candidate_id = candidate
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let current_id = current
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    candidate_created > current_created
        || (candidate_created == current_created && candidate_id < current_id)
}

fn workflow_owner(event: &serde_json::Value) -> Option<String> {
    if event.get("kind").and_then(|value| value.as_u64()) == Some(30623) {
        json_tag(event, "owner")
    } else {
        event.get("pubkey")?.as_str().map(str::to_string)
    }
}

fn workflow_revision(event: &serde_json::Value) -> Option<String> {
    if event.get("kind").and_then(|value| value.as_u64()) == Some(30623) {
        json_tag(event, "e")
    } else {
        event.get("id")?.as_str().map(str::to_string)
    }
}

/// List workflows in a channel using relay state with a legacy fallback.
pub async fn cmd_list_workflows(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [30623, 30620],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let workflows: Vec<serde_json::Value> = current_workflow_events(&events)
        .iter()
        .map(|e| {
            serde_json::json!({
                "workflow_id": extract_d_tag(e),
                "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "pubkey": workflow_owner(e).unwrap_or_default(),
                "revision": workflow_revision(e).unwrap_or_default(),
            })
        })
        .collect();
    let output = serde_json::to_string(&workflows).unwrap_or_default();
    println!("{output}");
    Ok(())
}

/// Get a single workflow definition.
pub async fn cmd_get_workflow(client: &BuzzClient, workflow_id: &str) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let filter = serde_json::json!({
        "kinds": [30623, 30620],
        "#d": [workflow_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    if let Some(e) = current_workflow_events(&events).first() {
        let normalized = serde_json::json!({
            "workflow_id": extract_d_tag(e),
            "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
            "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
            "pubkey": workflow_owner(e).unwrap_or_default(),
            "revision": workflow_revision(e).unwrap_or_default(),
        });
        println!("{normalized}");
    } else {
        println!("null");
    }
    Ok(())
}

/// Get workflow run history — query kinds [46001, 46002, 46003].
///
/// NOTE: The relay does not currently emit workflow execution events (46001-46003).
/// Run history is stored in the workflow_runs DB table, not as Nostr events.
/// This command will return an empty array until the relay adds event emission
/// or a dedicated REST endpoint for run history.
pub async fn cmd_get_workflow_runs(
    client: &BuzzClient,
    workflow_id: &str,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let limit = limit.unwrap_or(20).min(100);
    let filter = serde_json::json!({
        "kinds": [46001, 46002, 46003],
        "#d": [workflow_id],
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let normalized: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "event_id": e.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "kind": e.get("kind").and_then(|v| v.as_u64()).unwrap_or(0),
                "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "tags": e.get("tags").cloned().unwrap_or(serde_json::json!([])),
            })
        })
        .collect();
    let output = serde_json::to_string(&normalized).unwrap_or_default();
    println!("{output}");
    Ok(())
}

/// Create a workflow — sign and submit a kind:30620 event.
pub async fn cmd_create_workflow(
    client: &BuzzClient,
    channel_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

    let workflow_id = uuid::Uuid::new_v4();
    let builder = buzz_sdk::build_workflow_def(channel_uuid, workflow_id, &yaml_definition)
        .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    let final_workflow_id = extract_relay_response_field(&resp, "workflow_id")
        .unwrap_or_else(|| workflow_id.to_string());
    print_create_response(&resp, "workflow_id", &final_workflow_id);
    Ok(())
}

/// Update a workflow through the relay-authorized kind:46021 command.
pub async fn cmd_update_workflow(
    client: &BuzzClient,
    channel_id: &str,
    workflow_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let wf_uuid = parse_uuid(workflow_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

    let filter = serde_json::json!({
        "kinds": [30623, 30620],
        "#d": [workflow_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let current = current_workflow_events(&events)
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound(format!("workflow {workflow_id} not found")))?;
    let expected_revision = workflow_revision(&current)
        .ok_or_else(|| CliError::Other("workflow state is missing its revision".into()))?;
    let owner = workflow_owner(&current)
        .ok_or_else(|| CliError::Other("workflow state is missing its owner".into()))?;

    let builder = buzz_sdk::build_workflow_update_request(
        &owner,
        channel_uuid,
        wf_uuid,
        &yaml_definition,
        &expected_revision,
    )
    .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Delete a workflow — sign and submit a kind:5 deletion event.
pub async fn cmd_delete_workflow(client: &BuzzClient, workflow_id: &str) -> Result<(), CliError> {
    let wf_uuid = parse_uuid(workflow_id)?;
    let filter = serde_json::json!({
        "kinds": [30623, 30620],
        "#d": [workflow_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let current = current_workflow_events(&events)
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound(format!("workflow {workflow_id} not found")))?;
    let owner = workflow_owner(&current)
        .ok_or_else(|| CliError::Other("workflow state is missing its owner".into()))?;
    let expected_revision = workflow_revision(&current)
        .ok_or_else(|| CliError::Other("workflow state is missing its revision".into()))?;

    let builder =
        buzz_sdk::build_workflow_delete(&owner, wf_uuid, &expected_revision).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Trigger a workflow — sign and submit a kind:46020 event.
///
/// When `inputs` is provided, it is parsed as a JSON object and used as the
/// event content (MCP parity). When omitted, the event content is `{}`.
pub async fn cmd_trigger_workflow(
    client: &BuzzClient,
    workflow_id: &str,
    inputs: Option<&str>,
) -> Result<(), CliError> {
    let wf_uuid = parse_uuid(workflow_id)?;

    if let Some(raw) = inputs {
        // Parse and validate it is a JSON object, then build the event manually
        // so we can embed the inputs as the event content.
        let parsed: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| CliError::Usage(format!("--inputs is not valid JSON: {e}")))?;
        if !parsed.is_object() {
            return Err(CliError::Usage("--inputs must be a JSON object".into()));
        }
        let content = serde_json::to_string(&parsed).unwrap_or_default();
        use nostr::{EventBuilder, Kind, Tag};
        let tags = vec![Tag::parse(["d", &wf_uuid.to_string()])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?];
        let builder = EventBuilder::new(
            Kind::Custom(buzz_sdk::kind::KIND_WORKFLOW_TRIGGER as u16),
            &content,
        )
        .tags(tags);
        let event = client.sign_event(builder)?;
        let resp = client.submit_event(event).await?;
        println!("{}", normalize_write_response(&resp));
    } else {
        let builder = buzz_sdk::build_workflow_trigger(wf_uuid).map_err(sdk_err)?;
        let event = client.sign_event(builder)?;
        let resp = client.submit_event(event).await?;
        println!("{}", normalize_write_response(&resp));
    }
    Ok(())
}

/// Approve or deny a workflow step — sign and submit a kind:46030 (grant) or 46031 (deny) event.
pub async fn cmd_approve_step(
    client: &BuzzClient,
    approval_token: &str,
    approved: bool,
    note: Option<&str>,
) -> Result<(), CliError> {
    validate_uuid(approval_token)?;

    let content = note.unwrap_or("");

    // The relay expects d-tag = hex(SHA256(token)), not the raw token UUID.
    let token_hash = hex::encode(Sha256::digest(approval_token.as_bytes()));
    let builder =
        buzz_sdk::build_workflow_approval(&token_hash, approved, content).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(cmd: crate::WorkflowsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::WorkflowsCmd;
    match cmd {
        WorkflowsCmd::List { channel } => cmd_list_workflows(client, &channel).await,
        WorkflowsCmd::Get { workflow } => cmd_get_workflow(client, &workflow).await,
        WorkflowsCmd::Create { channel, yaml } => {
            cmd_create_workflow(client, &channel, &yaml).await
        }
        WorkflowsCmd::Update {
            channel,
            workflow,
            yaml,
        } => cmd_update_workflow(client, &channel, &workflow, &yaml).await,
        WorkflowsCmd::Delete { workflow } => cmd_delete_workflow(client, &workflow).await,
        WorkflowsCmd::Trigger { workflow, inputs } => {
            cmd_trigger_workflow(client, &workflow, inputs.as_deref()).await
        }
        WorkflowsCmd::Runs { workflow, limit } => {
            cmd_get_workflow_runs(client, &workflow, limit).await
        }
        WorkflowsCmd::Approve {
            token,
            approved,
            note,
        } => {
            // approved is already a bool — no parse_bool_flag needed
            cmd_approve_step(client, &token, approved, note.as_deref()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_state_wins_and_deleted_state_suppresses_legacy() {
        let id = uuid::Uuid::new_v4().to_string();
        let legacy = serde_json::json!({
            "id": "aa", "kind": 30620, "pubkey": "11", "content": "old",
            "tags": [["d", id.clone()], ["h", "channel"]]
        });
        let active = serde_json::json!({
            "id": "bb", "kind": 30623, "pubkey": "relay", "content": "new",
            "tags": [["d", id.clone()], ["h", "channel"], ["owner", "11"], ["e", "cc"], ["status", "active"]]
        });
        let current = current_workflow_events(&[legacy.clone(), active.clone()]);
        assert_eq!(current, vec![active]);
        assert_eq!(workflow_owner(&current[0]).as_deref(), Some("11"));
        assert_eq!(workflow_revision(&current[0]).as_deref(), Some("cc"));

        let deleted = serde_json::json!({
            "id": "dd", "kind": 30623, "pubkey": "relay", "content": "",
            "tags": [["d", id], ["status", "deleted"]]
        });
        assert!(current_workflow_events(&[legacy, deleted]).is_empty());
    }

    #[test]
    fn relay_state_fold_selects_one_head_after_key_rotation() {
        let id = uuid::Uuid::new_v4().to_string();
        let old = serde_json::json!({
            "id": "bb", "kind": 30623, "pubkey": "old-relay", "created_at": 100, "content": "old",
            "tags": [["d", id.clone()], ["owner", "11"], ["e", "aa"], ["status", "active"]]
        });
        let current = serde_json::json!({
            "id": "dd", "kind": 30623, "pubkey": "new-relay", "created_at": 200, "content": "new",
            "tags": [["d", id], ["owner", "11"], ["e", "cc"], ["status", "active"]]
        });

        assert_eq!(
            current_workflow_events(&[old, current.clone()]),
            vec![current]
        );
    }
}
