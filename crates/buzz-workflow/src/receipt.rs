//! Authenticated processing of durable agent checkpoint and artifact receipts.

use buzz_core::kind::{KIND_AGENT_WORKFLOW_ARTIFACT, KIND_AGENT_WORKFLOW_CHECKPOINT};
use buzz_core::tenant::CommunityId;
use buzz_db::agent_workflow::{AgentTask, AgentTaskStatus, CreateAgentArtifact};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{WorkflowEngine, WorkflowError};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointReceipt {
    sequence: i64,
    state: Value,
    #[serde(default)]
    artifact_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReceipt {
    kind: String,
    version: i32,
    content_type: String,
    sha256: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    inline_content: Option<Value>,
    #[serde(default = "empty_object")]
    metadata: Value,
}

#[derive(Debug, Clone, Copy)]
struct ReceiptCoordinates {
    workflow_id: Uuid,
    run_id: Uuid,
    task_id: Uuid,
}

fn empty_object() -> Value {
    json!({})
}

/// Process one already-accepted, channel-scoped agent receipt.
///
/// The event signer must be the task's persisted assignee. Invalid receipts
/// fail before any durable mutation and never complete a task.
pub async fn process_agent_receipt(
    engine: &WorkflowEngine,
    community: CommunityId,
    event: &buzz_core::StoredEvent,
) -> Result<(), WorkflowError> {
    let kind = buzz_core::kind::event_kind_u32(&event.event);
    if !matches!(
        kind,
        KIND_AGENT_WORKFLOW_CHECKPOINT | KIND_AGENT_WORKFLOW_ARTIFACT
    ) {
        return Ok(());
    }
    let channel_id = event
        .channel_id
        .ok_or_else(|| WorkflowError::Unauthorized("agent receipt has no channel scope".into()))?;
    let coordinates = receipt_coordinates(&event.event)?;
    let run = engine
        .db
        .get_workflow_run(community, coordinates.run_id)
        .await?;
    if run.workflow_id != coordinates.workflow_id {
        return Err(WorkflowError::Unauthorized(
            "agent receipt workflow tag does not match its durable run".into(),
        ));
    }
    let store = engine.db.agent_workflow_store();
    let run_state = store
        .get_run_state(community, coordinates.run_id)
        .await?
        .ok_or_else(|| {
            WorkflowError::Unauthorized("agent receipt run state was not found".into())
        })?;
    let expected_channel = run_state
        .metadata
        .get("channel_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| {
            WorkflowError::Unauthorized(
                "agent receipt run state has no valid channel binding".into(),
            )
        })?;
    if expected_channel != channel_id {
        return Err(WorkflowError::Unauthorized(
            "agent receipt channel does not match its durable run".into(),
        ));
    }
    let task = store
        .get_task(community, coordinates.task_id)
        .await?
        .ok_or_else(|| WorkflowError::Unauthorized("agent receipt task was not found".into()))?;
    authenticate_receipt(
        &task,
        coordinates.run_id,
        &event.event,
        kind == KIND_AGENT_WORKFLOW_ARTIFACT,
    )?;

    match kind {
        KIND_AGENT_WORKFLOW_CHECKPOINT => {
            let receipt: CheckpointReceipt = parse_content(&event.event.content)?;
            if receipt.sequence <= 0 {
                return Err(WorkflowError::InvalidDefinition(
                    "checkpoint sequence must be positive".into(),
                ));
            }
            store
                .append_checkpoint(
                    community,
                    coordinates.run_id,
                    coordinates.task_id,
                    receipt.sequence,
                    &receipt.state,
                    receipt.artifact_id,
                )
                .await?;
        }
        KIND_AGENT_WORKFLOW_ARTIFACT => {
            let receipt: ArtifactReceipt = parse_content(&event.event.content)?;
            validate_artifact_receipt(&task, &receipt)?;
            let digest = decode_sha256(&receipt.sha256)?;
            let event_id = event.event.id.to_hex();
            if let Some((completed, artifact)) = store
                .persist_artifact_and_complete(
                    community,
                    task.id,
                    task.version,
                    CreateAgentArtifact {
                        run_id: coordinates.run_id,
                        task_id: Some(coordinates.task_id),
                        kind: &receipt.kind,
                        version: receipt.version,
                        content_type: &receipt.content_type,
                        uri: receipt.uri.as_deref(),
                        sha256: &digest,
                        inline_content: receipt.inline_content.as_ref(),
                        metadata: &receipt.metadata,
                        created_by: Some(event.event.pubkey.as_bytes()),
                        idempotency_key: &event_id,
                    },
                )
                .await?
            {
                store
                    .append_next_transition(
                        community,
                        coordinates.run_id,
                        Some(&task.phase),
                        &completed.phase,
                        Some(&task.status.to_string()),
                        &completed.status.to_string(),
                        Some("validated artifact receipt completed task"),
                        Some(event.event.pubkey.as_bytes()),
                        &json!({
                            "task_id": completed.id,
                            "task_key": completed.task_key,
                            "task_version": completed.version,
                            "artifact_id": artifact.id,
                            "artifact_event_id": event_id,
                        }),
                    )
                    .await?;
                crate::durable_settlement::advance_settle_and_project(
                    engine,
                    community,
                    coordinates.run_id,
                )
                .await?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn receipt_coordinates(event: &nostr::Event) -> Result<ReceiptCoordinates, WorkflowError> {
    let run_id = unique_uuid_tag(event, "run")?;
    let d = unique_uuid_tag(event, "d")?;
    if d != run_id {
        return Err(WorkflowError::Unauthorized(
            "agent receipt d and run tags differ".into(),
        ));
    }
    Ok(ReceiptCoordinates {
        workflow_id: unique_uuid_tag(event, "workflow")?,
        run_id,
        task_id: unique_uuid_tag(event, "task")?,
    })
}

fn unique_uuid_tag(event: &nostr::Event, name: &str) -> Result<Uuid, WorkflowError> {
    let mut values = event.tags.iter().filter_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some(name))
            .then(|| parts.get(1).cloned())
            .flatten()
    });
    let value = values.next().ok_or_else(|| {
        WorkflowError::Unauthorized(format!("agent receipt is missing the {name} tag"))
    })?;
    if values.next().is_some() {
        return Err(WorkflowError::Unauthorized(format!(
            "agent receipt has duplicate {name} tags"
        )));
    }
    Uuid::parse_str(&value).map_err(|error| {
        WorkflowError::Unauthorized(format!("agent receipt has invalid {name} tag: {error}"))
    })
}

fn authenticate_receipt(
    task: &AgentTask,
    run_id: Uuid,
    event: &nostr::Event,
    allow_completed: bool,
) -> Result<(), WorkflowError> {
    if task.run_id != run_id {
        return Err(WorkflowError::Unauthorized(
            "agent receipt task belongs to another run".into(),
        ));
    }
    if !(matches!(
        task.status,
        AgentTaskStatus::Running | AgentTaskStatus::Waiting
    ) || allow_completed && task.status == AgentTaskStatus::Completed)
    {
        return Err(WorkflowError::Unauthorized(format!(
            "agent receipt task is not active: {}",
            task.status
        )));
    }
    let assignee = task.agent_pubkey.as_deref().ok_or_else(|| {
        WorkflowError::Unauthorized("agent receipt task has no persisted assignee".into())
    })?;
    if assignee != event.pubkey.as_bytes() {
        return Err(WorkflowError::Unauthorized(
            "agent receipt signer is not the persisted task assignee".into(),
        ));
    }
    Ok(())
}

fn parse_content<T: for<'de> Deserialize<'de>>(content: &str) -> Result<T, WorkflowError> {
    serde_json::from_str(content).map_err(|error| {
        WorkflowError::InvalidDefinition(format!("invalid agent receipt content: {error}"))
    })
}

fn validate_artifact_receipt(
    task: &AgentTask,
    receipt: &ArtifactReceipt,
) -> Result<(), WorkflowError> {
    if receipt.kind != task.task_key {
        return Err(WorkflowError::InvalidDefinition(
            "artifact kind must equal the durable task key".into(),
        ));
    }
    if receipt.version != 1 {
        return Err(WorkflowError::InvalidDefinition(
            "one-output durable tasks require artifact version 1".into(),
        ));
    }
    if receipt.content_type != "application/json" {
        return Err(WorkflowError::InvalidDefinition(
            "agent artifacts must use application/json".into(),
        ));
    }
    if receipt.uri.is_none() && receipt.inline_content.is_none() {
        return Err(WorkflowError::InvalidDefinition(
            "artifact requires uri or inline_content".into(),
        ));
    }
    if !receipt.metadata.is_object() {
        return Err(WorkflowError::InvalidDefinition(
            "artifact metadata must be a JSON object".into(),
        ));
    }
    let inline = receipt.inline_content.as_ref().ok_or_else(|| {
        WorkflowError::InvalidDefinition(
            "schema-validated agent artifact requires inline_content".into(),
        )
    })?;
    let canonical = serde_json::to_vec(inline)
        .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
    let actual = hex::encode(Sha256::digest(&canonical));
    if !actual.eq_ignore_ascii_case(&receipt.sha256) {
        return Err(WorkflowError::InvalidDefinition(
            "artifact sha256 does not match canonical inline_content".into(),
        ));
    }
    let schema = task.output_schema.as_ref().ok_or_else(|| {
        WorkflowError::InvalidDefinition("agent task has no output schema".into())
    })?;
    let validator = jsonschema::validator_for(schema).map_err(|error| {
        WorkflowError::InvalidDefinition(format!("persisted output schema is invalid: {error}"))
    })?;
    validator.validate(inline).map_err(|error| {
        WorkflowError::InvalidDefinition(format!("artifact failed output schema: {error}"))
    })?;
    Ok(())
}

fn decode_sha256(value: &str) -> Result<Vec<u8>, WorkflowError> {
    let bytes = hex::decode(value).map_err(|error| {
        WorkflowError::InvalidDefinition(format!("artifact sha256 is not hex: {error}"))
    })?;
    if bytes.len() != 32 {
        return Err(WorkflowError::InvalidDefinition(
            "artifact sha256 must contain 32 bytes".into(),
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(schema: Value) -> AgentTask {
        let now = chrono::Utc::now();
        AgentTask {
            id: Uuid::nil(),
            run_id: Uuid::nil(),
            task_key: "analysis".into(),
            phase: "analysis".into(),
            agent_pubkey: Some(vec![1; 32]),
            status: AgentTaskStatus::Running,
            attempt: 1,
            max_attempts: 2,
            input: json!({}),
            output_schema: Some(schema),
            idempotency_key: "run:analysis".into(),
            parent_task_id: None,
            depends_on: json!([]),
            not_before: None,
            started_at: Some(now),
            completed_at: None,
            error_code: None,
            error_message: None,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    fn artifact(inline: Value) -> ArtifactReceipt {
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&inline).expect("test JSON must serialize"),
        ));
        ArtifactReceipt {
            kind: "analysis".into(),
            version: 1,
            content_type: "application/json".into(),
            sha256: digest,
            uri: None,
            inline_content: Some(inline),
            metadata: json!({}),
        }
    }

    #[test]
    fn valid_artifact_passes_hash_and_schema() {
        let task = task(json!({
            "type": "object",
            "required": ["decision"],
            "properties": { "decision": { "type": "string", "minLength": 1 } }
        }));
        assert!(
            validate_artifact_receipt(&task, &artifact(json!({ "decision": "granted" }))).is_ok()
        );
    }

    #[test]
    fn invalid_hash_or_schema_is_rejected() {
        let task = task(json!({
            "type": "object",
            "required": ["decision"]
        }));
        let mut bad_hash = artifact(json!({ "decision": "granted" }));
        bad_hash.sha256 = "00".repeat(32);
        assert!(validate_artifact_receipt(&task, &bad_hash).is_err());
        assert!(validate_artifact_receipt(&task, &artifact(json!({}))).is_err());
    }
}
