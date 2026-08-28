//! Durable DAG materialization and fire-and-forget agent dispatch.

use std::collections::HashMap;

use base64::Engine as _;
use buzz_core::tenant::CommunityId;
use buzz_db::agent_workflow::{AgentTask, AgentTaskStatus, CreateAgentTask, EnsureAgentRunState};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::executor::{resolve_template, ExecutionDisposition, ExecutionResult, TriggerContext};
use crate::schema::{ActionDef, Step, WorkflowDef};
use crate::{WorkflowEngine, WorkflowError};

const DEFAULT_MAX_ATTEMPTS: i32 = 3;
const DEFAULT_AGENT_TIMEOUT_SECONDS: u64 = 300;
const DISPATCH_RETRY_DELAY_SECONDS: i64 = 30;

/// Result of one idempotent scheduler advancement pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DurableProgress {
    /// Tasks materialized by definition (existing rows included).
    pub task_count: usize,
    /// Agent tasks claimed and invited in this pass.
    pub dispatched: usize,
    /// Coordinator barriers completed in this pass.
    pub barriers_completed: usize,
    /// Tasks blocked because their roster identity was invalid or absent.
    pub blocked: usize,
    /// Whether every durable task is complete.
    pub all_complete: bool,
    /// Whether at least one durable task reached a terminal failure state.
    pub terminal_failure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableTaskInput {
    channel_id: Uuid,
    step_index: i32,
    #[serde(default = "default_agent_timeout_seconds")]
    timeout_secs: u64,
    trigger: TriggerContext,
    action: ActionDef,
}

const fn default_agent_timeout_seconds() -> u64 {
    DEFAULT_AGENT_TIMEOUT_SECONDS
}

struct TaskBlueprint {
    input: Value,
    agent_pubkey: Option<Vec<u8>>,
    output_schema: Option<Value>,
    depends_on: Value,
    idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentPayload {
    source_name: String,
    content_type: String,
    source_base64: String,
    pages: Vec<DocumentPayloadPage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentPayloadPage {
    physical_page: u32,
    #[serde(default)]
    logical_label: Option<String>,
    text: String,
}

#[derive(Debug)]
struct PreparedIngestion {
    content: Value,
    artifact_sha256: Vec<u8>,
    manifest_hash: Vec<u8>,
    metadata: Value,
}

/// Materialize a durable definition and perform one non-blocking advancement pass.
///
/// This function never waits for an agent response. It returns after signed task
/// invitations are published; output arrives later as an agent artifact receipt.
pub async fn execute_durable_run(
    engine: &WorkflowEngine,
    community: CommunityId,
    run_id: Uuid,
    definition: &WorkflowDef,
    trigger: &TriggerContext,
) -> Result<ExecutionResult, WorkflowError> {
    engine
        .db
        .update_workflow_run(
            community,
            run_id,
            buzz_db::workflow::RunStatus::Running,
            0,
            &json!([]),
            None,
        )
        .await?;
    materialize_run(engine, community, run_id, definition, trigger).await?;
    let progress =
        crate::durable_settlement::advance_settle_and_project(engine, community, run_id).await?;
    Ok(ExecutionResult {
        disposition: if progress.all_complete || progress.terminal_failure {
            ExecutionDisposition::DurableSettled
        } else {
            ExecutionDisposition::WaitingDurableTasks
        },
        approval_token: None,
        step_index: progress.task_count,
        step_outputs: HashMap::new(),
        trace: vec![json!({
            "status": if progress.all_complete { "completed" } else { "waiting_durable_tasks" },
            "task_count": progress.task_count,
            "dispatched": progress.dispatched,
            "barriers_completed": progress.barriers_completed,
            "blocked": progress.blocked,
        })],
    })
}

/// Idempotently create run state and one durable task per workflow step.
pub async fn materialize_run(
    engine: &WorkflowEngine,
    community: CommunityId,
    run_id: Uuid,
    definition: &WorkflowDef,
    trigger: &TriggerContext,
) -> Result<(), WorkflowError> {
    let channel_id = Uuid::parse_str(&trigger.channel_id).map_err(|error| {
        WorkflowError::InvalidDefinition(format!(
            "durable workflow trigger channel is not a UUID: {error}"
        ))
    })?;
    let store = engine.db.agent_workflow_store();
    let metadata = json!({
        "workflow_name": definition.name,
        "channel_id": channel_id,
        "trigger": sanitized_trigger(trigger),
    });
    store
        .ensure_run_state(
            community,
            EnsureAgentRunState {
                run_id,
                phase: "materialized",
                manifest_hash: None,
                thread_root_event_id: None,
                deadline: None,
                metadata: &metadata,
            },
        )
        .await?;

    for (step_index, step) in definition.steps.iter().enumerate() {
        let step_index = i32::try_from(step_index).map_err(|_| {
            WorkflowError::InvalidDefinition("workflow step index exceeds i32".into())
        })?;
        let blueprint = task_blueprint(run_id, channel_id, trigger, step_index, step)?;
        let persisted = store
            .create_task(
                community,
                CreateAgentTask {
                    run_id,
                    task_key: &step.id,
                    phase: &step.id,
                    agent_pubkey: blueprint.agent_pubkey.as_deref(),
                    max_attempts: DEFAULT_MAX_ATTEMPTS,
                    input: &blueprint.input,
                    output_schema: blueprint.output_schema.as_ref(),
                    idempotency_key: &blueprint.idempotency_key,
                    parent_task_id: None,
                    depends_on: &blueprint.depends_on,
                },
            )
            .await?;
        if persisted.task_key != step.id
            || persisted.phase != step.id
            || persisted.agent_pubkey != blueprint.agent_pubkey
            || persisted.max_attempts != DEFAULT_MAX_ATTEMPTS
            || persisted.input != blueprint.input
            || persisted.output_schema != blueprint.output_schema
            || persisted.parent_task_id.is_some()
            || persisted.depends_on != blueprint.depends_on
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "durable task '{}' conflicts with its persisted blueprint",
                step.id
            )));
        }
    }

    store
        .append_transition(
            community,
            run_id,
            0,
            None,
            "materialized",
            Some("pending"),
            "running",
            Some("durable DAG materialized"),
            None,
            &json!({ "task_count": definition.steps.len() }),
        )
        .await?;
    Ok(())
}

/// Advance all currently eligible barriers and agent tasks once.
///
/// Eligibility is rechecked atomically by the database transition itself. A
/// stale Rust snapshot can therefore cause a harmless no-op, never early fan-in.
pub async fn advance_run(
    engine: &WorkflowEngine,
    community: CommunityId,
    run_id: Uuid,
) -> Result<DurableProgress, WorkflowError> {
    let store = engine.db.agent_workflow_store();
    let mut progress = DurableProgress::default();
    let mut iterations = 0_usize;

    loop {
        iterations += 1;
        if iterations > 4_096 {
            tracing::warn!(run_id = %run_id, "durable advancement iteration limit reached");
            return Ok(progress);
        }
        let tasks = store.list_tasks(community, run_id, Some(1_000)).await?;
        progress.task_count = tasks.len();
        let mut changed = false;

        for task in tasks
            .iter()
            .filter(|task| task.status == AgentTaskStatus::Running)
        {
            let input = parse_task_input(task)?;
            if !matches!(
                input.action,
                ActionDef::RunAgent { .. } | ActionDef::VerifyArtifact { .. }
            ) {
                continue;
            }
            let timeout_secs = i64::try_from(input.timeout_secs).map_err(|_| {
                WorkflowError::InvalidDefinition(format!(
                    "durable task '{}' timeout exceeds i64",
                    task.task_key
                ))
            })?;
            let retry_at = Utc::now() + Duration::seconds(DISPATCH_RETRY_DELAY_SECONDS);
            if let Some(recovered) = store
                .recover_timed_out_task(community, task.id, task.version, timeout_secs, retry_at)
                .await?
            {
                append_task_transition(
                    &store,
                    community,
                    run_id,
                    task,
                    &recovered,
                    if recovered.status == AgentTaskStatus::Failed {
                        "agent attempt timed out and exhausted retries"
                    } else {
                        "agent attempt timed out and scheduled retry"
                    },
                    task.agent_pubkey.as_deref(),
                )
                .await?;
                changed = true;
            }
        }

        let now = Utc::now();
        for task in tasks.iter().filter(|task| match task.status {
            AgentTaskStatus::Pending => true,
            AgentTaskStatus::RetryScheduled => task
                .not_before
                .as_ref()
                .is_none_or(|not_before| not_before <= &now),
            _ => false,
        }) {
            let input = parse_task_input(task)?;
            match &input.action {
                ActionDef::Barrier => {
                    if let Some(completed) = store
                        .complete_ready_task(community, task.id, task.version)
                        .await?
                    {
                        append_task_transition(
                            &store,
                            community,
                            run_id,
                            task,
                            &completed,
                            "barrier dependencies completed",
                            None,
                        )
                        .await?;
                        progress.barriers_completed += 1;
                        changed = true;
                    }
                }
                ActionDef::RunAgent { prompt, .. } | ActionDef::VerifyArtifact { prompt, .. } => {
                    let dispatcher = engine.agent_dispatch()?;
                    let assignee = task.agent_pubkey.as_deref().ok_or_else(|| {
                        WorkflowError::InvalidDefinition(format!(
                            "durable agent task '{}' has no explicit identity",
                            task.task_key
                        ))
                    })?;
                    let Some(claimed) = store
                        .claim_task(community, task.id, task.version, assignee)
                        .await?
                    else {
                        continue;
                    };
                    append_task_transition(
                        &store,
                        community,
                        run_id,
                        task,
                        &claimed,
                        "agent task claimed",
                        Some(assignee),
                    )
                    .await?;
                    let prompt = resolve_template(prompt, &input.trigger, &HashMap::new())?;
                    let checkpoint = store
                        .latest_checkpoint(community, run_id, claimed.id)
                        .await?
                        .map(|checkpoint| checkpoint.state);
                    match dispatcher
                        .dispatch_task(
                            community,
                            input.channel_id,
                            run_id,
                            claimed.id,
                            assignee,
                            &prompt,
                            claimed.output_schema.as_ref(),
                            checkpoint.as_ref(),
                        )
                        .await
                    {
                        Ok(_) => progress.dispatched += 1,
                        Err(error) if claimed.attempt >= claimed.max_attempts => {
                            if let Some(failed) = store
                                .fail_task(
                                    community,
                                    claimed.id,
                                    claimed.version,
                                    false,
                                    "dispatch_attempts_exhausted",
                                    &error.to_string(),
                                )
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    &claimed,
                                    &failed,
                                    "agent dispatch attempts exhausted",
                                    None,
                                )
                                .await?;
                            }
                        }
                        Err(error) => {
                            let retry_at =
                                Utc::now() + Duration::seconds(DISPATCH_RETRY_DELAY_SECONDS);
                            if let Some(retry) = store
                                .schedule_retry(
                                    community,
                                    claimed.id,
                                    claimed.version,
                                    retry_at,
                                    "dispatch_failed",
                                    &error.to_string(),
                                )
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    &claimed,
                                    &retry,
                                    "agent dispatch scheduled for retry",
                                    None,
                                )
                                .await?;
                            }
                        }
                    }
                    changed = true;
                }
                ActionDef::IngestDocument { source, output } => match prepare_ingestion(source) {
                    Ok(prepared) => {
                        if let Some((completed, _artifact)) = store
                            .complete_ingestion_task(
                                community,
                                task.id,
                                task.version,
                                output,
                                &prepared.artifact_sha256,
                                &prepared.manifest_hash,
                                &prepared.content,
                                &prepared.metadata,
                                &format!("{}:manifest", task.idempotency_key),
                            )
                            .await?
                        {
                            append_task_transition(
                                &store,
                                community,
                                run_id,
                                task,
                                &completed,
                                "document manifest ingested",
                                None,
                            )
                            .await?;
                            changed = true;
                        }
                    }
                    Err(message) => {
                        if let Some(blocked) = store
                            .block_ready_task(
                                community,
                                task.id,
                                task.version,
                                "invalid_document_input",
                                &message,
                            )
                            .await?
                        {
                            append_task_transition(
                                &store,
                                community,
                                run_id,
                                task,
                                &blocked,
                                "document ingestion input rejected",
                                None,
                            )
                            .await?;
                            progress.blocked += 1;
                            changed = true;
                        }
                    }
                },
                ActionDef::PublishArtifact { artifact } => {
                    if !dependencies_complete(task, &tasks)? {
                        continue;
                    }
                    let producer = tasks
                        .iter()
                        .find(|candidate| candidate.task_key == *artifact)
                        .ok_or_else(|| {
                            WorkflowError::InvalidDefinition(format!(
                                "publish task '{}' references unknown artifact task '{artifact}'",
                                task.task_key
                            ))
                        })?;
                    if producer.status != AgentTaskStatus::Completed {
                        continue;
                    }
                    let artifacts = store.list_artifacts(community, run_id, Some(1_000)).await?;
                    let matching = artifacts
                        .iter()
                        .filter(|candidate| {
                            candidate.task_id == Some(producer.id)
                                && candidate.kind == *artifact
                                && candidate.version == 1
                        })
                        .collect::<Vec<_>>();
                    let [source] = matching.as_slice() else {
                        return Err(WorkflowError::InvalidDefinition(format!(
                            "completed task '{artifact}' must have exactly one version-1 artifact"
                        )));
                    };
                    let created_at_secs =
                        u64::try_from(task.created_at.timestamp()).map_err(|_| {
                            WorkflowError::InvalidDefinition(
                                "publish task creation timestamp precedes Unix epoch".into(),
                            )
                        })?;
                    let publication = json!({
                        "type": "approved_artifact_published",
                        "publish_task_id": task.id,
                        "source_task_id": producer.id,
                        "artifact_id": source.id,
                        "artifact_kind": source.kind,
                        "artifact_version": source.version,
                        "content_type": source.content_type,
                        "sha256": hex::encode(&source.sha256),
                        "uri": source.uri,
                        "inline_content": source.inline_content,
                        "metadata": source.metadata,
                        "approved": true,
                    });
                    match engine
                        .agent_dispatch()?
                        .publish_artifact(
                            community,
                            input.channel_id,
                            run_id,
                            task.id,
                            created_at_secs,
                            source.created_by.as_deref(),
                            &publication,
                        )
                        .await
                    {
                        Ok(event_id) => {
                            if let Some(completed) = store
                                .complete_ready_task(community, task.id, task.version)
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    task,
                                    &completed,
                                    "approved artifact published",
                                    None,
                                )
                                .await?;
                                store
                                    .append_next_transition(
                                        community,
                                        run_id,
                                        Some(&task.phase),
                                        &completed.phase,
                                        Some("published"),
                                        "published",
                                        Some("coordinator-signed artifact projection persisted"),
                                        None,
                                        &json!({
                                            "task_id": completed.id,
                                            "source_task_id": producer.id,
                                            "artifact_id": source.id,
                                            "publication_event_id": event_id,
                                        }),
                                    )
                                    .await?;
                                changed = true;
                            }
                        }
                        Err(error) => {
                            let retry_at =
                                Utc::now() + Duration::seconds(DISPATCH_RETRY_DELAY_SECONDS);
                            if let Some(deferred) = store
                                .defer_ready_task(
                                    community,
                                    task.id,
                                    task.version,
                                    retry_at,
                                    "artifact_publication_failed",
                                    &error.to_string(),
                                )
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    task,
                                    &deferred,
                                    if deferred.status == AgentTaskStatus::Failed {
                                        "artifact publication attempts exhausted"
                                    } else {
                                        "artifact publication scheduled for retry"
                                    },
                                    None,
                                )
                                .await?;
                                changed = true;
                            }
                        }
                    }
                }
                ActionDef::RequestApproval {
                    from,
                    message,
                    timeout,
                } => {
                    let run = engine.db.get_workflow_run(community, run_id).await?;
                    let workflow = engine.db.get_workflow(community, run.workflow_id).await?;
                    let approver_spec = resolve_approver_spec(from, &workflow.owner_pubkey)?;
                    let timeout_secs =
                        crate::executor::parse_duration_secs(timeout.as_deref().unwrap_or("24h"))?;
                    let timeout_secs = i64::try_from(timeout_secs).map_err(|_| {
                        WorkflowError::InvalidDefinition("approval timeout exceeds i64".into())
                    })?;
                    let expires_at = Utc::now() + Duration::seconds(timeout_secs);
                    let Some(approval) = store
                        .ensure_approval(
                            community,
                            buzz_db::agent_approval::EnsureAgentApproval {
                                workflow_id: run.workflow_id,
                                run_id,
                                task_id: task.id,
                                step_id: &task.task_key,
                                request_message: message,
                                step_index: input.step_index,
                                approver_spec: &approver_spec,
                                expires_at,
                            },
                        )
                        .await?
                    else {
                        continue;
                    };
                    match approval.status {
                        buzz_db::workflow::ApprovalStatus::Pending
                            if Utc::now() >= approval.expires_at =>
                        {
                            store.expire_approval(community, run_id, task.id).await?;
                            if let Some(blocked) = store
                                .block_ready_task(
                                    community,
                                    task.id,
                                    task.version,
                                    "approval_expired",
                                    "human approval deadline elapsed",
                                )
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    task,
                                    &blocked,
                                    "human approval expired",
                                    None,
                                )
                                .await?;
                                changed = true;
                            }
                        }
                        buzz_db::workflow::ApprovalStatus::Pending => {
                            if store
                                .mark_run_waiting_approval(community, run_id, input.step_index)
                                .await?
                            {
                                store
                                    .append_next_transition(
                                        community,
                                        run_id,
                                        Some(&task.phase),
                                        &task.phase,
                                        Some("running"),
                                        "waiting_approval",
                                        Some("human approval requested"),
                                        None,
                                        &json!({
                                            "task_id": task.id,
                                            "task_key": task.task_key,
                                            "approval_ref": hex::encode(&approval.token),
                                            "request_message": approval.request_message,
                                            "approver": approval.approver_spec,
                                            "expires_at": approval.expires_at,
                                        }),
                                    )
                                    .await?;
                            }
                        }
                        buzz_db::workflow::ApprovalStatus::Granted => {
                            store
                                .mark_run_running_after_approval(community, run_id)
                                .await?;
                            if let Some(completed) = store
                                .complete_ready_task(community, task.id, task.version)
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    task,
                                    &completed,
                                    "human approval granted",
                                    approval.approver_pubkey.as_deref(),
                                )
                                .await?;
                                changed = true;
                            }
                        }
                        buzz_db::workflow::ApprovalStatus::Denied => {
                            if let Some(blocked) = store
                                .block_ready_task(
                                    community,
                                    task.id,
                                    task.version,
                                    "approval_denied",
                                    "human approver denied publication",
                                )
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    task,
                                    &blocked,
                                    "human approval denied",
                                    approval.approver_pubkey.as_deref(),
                                )
                                .await?;
                                changed = true;
                            }
                        }
                        buzz_db::workflow::ApprovalStatus::Expired => {
                            if let Some(blocked) = store
                                .block_ready_task(
                                    community,
                                    task.id,
                                    task.version,
                                    "approval_expired",
                                    "human approval deadline elapsed",
                                )
                                .await?
                            {
                                append_task_transition(
                                    &store,
                                    community,
                                    run_id,
                                    task,
                                    &blocked,
                                    "human approval expired",
                                    None,
                                )
                                .await?;
                                changed = true;
                            }
                        }
                    }
                }
                unsupported => {
                    let action = action_name(unsupported);
                    if let Some(blocked) = store
                        .block_ready_task(
                            community,
                            task.id,
                            task.version,
                            "action_not_implemented",
                            &format!("durable action '{action}' has no scheduler adapter"),
                        )
                        .await?
                    {
                        append_task_transition(
                            &store,
                            community,
                            run_id,
                            task,
                            &blocked,
                            "durable action has no scheduler adapter",
                            None,
                        )
                        .await?;
                        progress.blocked += 1;
                        changed = true;
                    }
                }
            }
        }

        if !changed {
            let final_tasks = store.list_tasks(community, run_id, Some(1_000)).await?;
            progress.task_count = final_tasks.len();
            progress.all_complete = !final_tasks.is_empty()
                && final_tasks
                    .iter()
                    .all(|task| task.status == AgentTaskStatus::Completed);
            progress.terminal_failure = final_tasks.iter().any(|task| {
                matches!(
                    task.status,
                    AgentTaskStatus::Failed | AgentTaskStatus::Cancelled | AgentTaskStatus::Blocked
                )
            });
            progress.blocked = final_tasks
                .iter()
                .filter(|task| task.status == AgentTaskStatus::Blocked)
                .count();
            return Ok(progress);
        }
    }
}

async fn append_task_transition(
    store: &buzz_db::agent_workflow_store::AgentWorkflowStore,
    community: CommunityId,
    run_id: Uuid,
    before: &AgentTask,
    after: &AgentTask,
    reason: &str,
    actor: Option<&[u8]>,
) -> Result<(), WorkflowError> {
    store
        .append_next_transition(
            community,
            run_id,
            Some(&before.phase),
            &after.phase,
            Some(&before.status.to_string()),
            &after.status.to_string(),
            Some(reason),
            actor,
            &json!({
                "task_id": after.id,
                "task_key": after.task_key,
                "task_version": after.version,
            }),
        )
        .await?;
    Ok(())
}

fn dependencies_complete(task: &AgentTask, tasks: &[AgentTask]) -> Result<bool, WorkflowError> {
    let dependencies = task.depends_on.as_array().ok_or_else(|| {
        WorkflowError::InvalidDefinition(format!(
            "durable task '{}' depends_on must be an array",
            task.task_key
        ))
    })?;
    Ok(dependencies.iter().all(|dependency| {
        dependency.as_str().is_some_and(|task_key| {
            tasks.iter().any(|candidate| {
                candidate.task_key == task_key && candidate.status == AgentTaskStatus::Completed
            })
        })
    }))
}

fn prepare_ingestion(source: &str) -> Result<PreparedIngestion, String> {
    let payload: DocumentPayload = serde_json::from_str(source)
        .map_err(|error| format!("document_input must be valid JSON: {error}"))?;
    let source_bytes = base64::engine::general_purpose::STANDARD
        .decode(&payload.source_base64)
        .map_err(|error| format!("document_input source_base64 is invalid: {error}"))?;
    let pages = payload
        .pages
        .into_iter()
        .map(|page| crate::document::ExtractedPage {
            physical_page: page.physical_page,
            logical_label: page.logical_label,
            text: page.text,
        })
        .collect::<Vec<_>>();
    let manifest = crate::document::build_document_manifest(
        crate::document::DocumentInput {
            source_name: &payload.source_name,
            content_type: &payload.content_type,
            source_bytes: &source_bytes,
            pages: &pages,
        },
        crate::document::IngestLimits::default(),
    )
    .map_err(|error| format!("document_input failed manifest validation: {error}"))?;
    let manifest_hash = hex::decode(&manifest.manifest_sha256)
        .map_err(|error| format!("manifest hash is invalid: {error}"))?;
    let content = serde_json::to_value(&manifest)
        .map_err(|error| format!("manifest serialization failed: {error}"))?;
    let canonical = serde_json::to_vec(&content)
        .map_err(|error| format!("manifest serialization failed: {error}"))?;
    let artifact_sha256 = Sha256::digest(canonical).to_vec();
    Ok(PreparedIngestion {
        content,
        artifact_sha256,
        manifest_hash,
        metadata: json!({
            "document_sha256": manifest.document_sha256,
            "manifest_sha256": manifest.manifest_sha256,
            "page_count": manifest.page_count,
            "chunk_count": manifest.chunks.len(),
        }),
    })
}

fn sanitized_trigger(trigger: &TriggerContext) -> TriggerContext {
    let mut sanitized = trigger.clone();
    sanitized.webhook_fields.remove("document_input");
    sanitized
}

fn resolve_approver_spec(from: &str, owner_pubkey: &[u8]) -> Result<String, WorkflowError> {
    if from == "@workflow-owner" {
        if owner_pubkey.len() != 32 {
            return Err(WorkflowError::InvalidDefinition(
                "workflow owner pubkey must contain 32 bytes".into(),
            ));
        }
        return Ok(hex::encode(owner_pubkey));
    }
    if from.len() == 64
        && from
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    {
        return Ok(from.to_owned());
    }
    Err(WorkflowError::InvalidDefinition(
        "durable approval from must be @workflow-owner or a lowercase 32-byte hex pubkey".into(),
    ))
}

fn action_name(action: &ActionDef) -> &'static str {
    match action {
        ActionDef::IngestDocument { .. } => "ingest_document",
        ActionDef::VerifyArtifact { .. } => "verify_artifact",
        ActionDef::RequestApproval { .. } => "request_approval",
        ActionDef::PublishArtifact { .. } => "publish_artifact",
        _ => "unsupported",
    }
}

fn task_blueprint(
    run_id: Uuid,
    channel_id: Uuid,
    trigger: &TriggerContext,
    step_index: i32,
    step: &Step,
) -> Result<TaskBlueprint, WorkflowError> {
    let action = match &step.action {
        ActionDef::IngestDocument { source, output } => ActionDef::IngestDocument {
            source: resolve_template(source, trigger, &HashMap::new())?,
            output: output.clone(),
        },
        _ => step.action.clone(),
    };
    let timeout_secs = step.timeout_secs.unwrap_or(DEFAULT_AGENT_TIMEOUT_SECONDS);
    if timeout_secs == 0 || i64::try_from(timeout_secs).is_err() {
        return Err(WorkflowError::InvalidDefinition(format!(
            "durable task '{}' timeout_secs must be between 1 and i64::MAX",
            step.id
        )));
    }
    let input = serde_json::to_value(DurableTaskInput {
        channel_id,
        step_index,
        timeout_secs,
        trigger: sanitized_trigger(trigger),
        action,
    })
    .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
    let agent_pubkey = match &step.action {
        ActionDef::RunAgent { identity, .. } | ActionDef::VerifyArtifact { identity, .. } => {
            Some(hex::decode(identity).map_err(|error| {
                WorkflowError::InvalidDefinition(format!(
                    "step '{}': invalid agent identity: {error}",
                    step.id
                ))
            })?)
        }
        _ => None,
    };
    let output_schema = match &step.action {
        ActionDef::RunAgent { output_schema, .. } => Some(output_schema.clone()),
        ActionDef::VerifyArtifact { schema, .. } => Some(schema.clone()),
        _ => None,
    };
    Ok(TaskBlueprint {
        input,
        agent_pubkey,
        output_schema,
        depends_on: json!(step.depends_on),
        idempotency_key: format!("{run_id}:{}", step.id),
    })
}

fn parse_task_input(task: &AgentTask) -> Result<DurableTaskInput, WorkflowError> {
    serde_json::from_value(task.input.clone()).map_err(|error| {
        WorkflowError::InvalidDefinition(format!(
            "durable task '{}' has invalid input: {error}",
            task.task_key
        ))
    })
}

#[cfg(test)]
#[path = "durable_tests.rs"]
mod tests;
