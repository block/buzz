//! Durable DAG materialization and fire-and-forget agent dispatch.

use std::collections::HashMap;

use buzz_core::tenant::CommunityId;
use buzz_db::agent_workflow::{AgentTask, AgentTaskStatus, CreateAgentTask, EnsureAgentRunState};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::executor::{resolve_template, ExecutionDisposition, ExecutionResult, TriggerContext};
use crate::schema::{ActionDef, Step, WorkflowDef};
use crate::{WorkflowEngine, WorkflowError};

const DEFAULT_MAX_ATTEMPTS: i32 = 3;
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
    trigger: TriggerContext,
    action: ActionDef,
}

struct TaskBlueprint {
    input: Value,
    agent_pubkey: Option<Vec<u8>>,
    output_schema: Option<Value>,
    depends_on: Value,
    idempotency_key: String,
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
    let progress = advance_run(engine, community, run_id).await?;
    if progress.terminal_failure {
        return Err(WorkflowError::NotImplemented(
            "one or more durable task actions have no scheduler adapter".into(),
        ));
    }
    Ok(ExecutionResult {
        disposition: if progress.all_complete {
            ExecutionDisposition::Completed
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
        "trigger": trigger,
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

    for step in &definition.steps {
        let blueprint = task_blueprint(run_id, channel_id, trigger, step)?;
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

    loop {
        let tasks = store.list_tasks(community, run_id, Some(1_000)).await?;
        progress.task_count = tasks.len();
        let mut changed = false;

        for task in tasks
            .iter()
            .filter(|task| task.status == AgentTaskStatus::Pending)
        {
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
                ActionDef::RunAgent { prompt, .. } => {
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
                        Err(error) => {
                            let retry_at =
                                Utc::now() + Duration::seconds(DISPATCH_RETRY_DELAY_SECONDS);
                            if store
                                .schedule_retry(
                                    community,
                                    claimed.id,
                                    claimed.version,
                                    retry_at,
                                    "dispatch_failed",
                                    &error.to_string(),
                                )
                                .await?
                                .is_none()
                            {
                                store
                                    .fail_task(
                                        community,
                                        claimed.id,
                                        claimed.version,
                                        false,
                                        "dispatch_failed",
                                        &error.to_string(),
                                    )
                                    .await?;
                            }
                        }
                    }
                    changed = true;
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
    step: &Step,
) -> Result<TaskBlueprint, WorkflowError> {
    let input = serde_json::to_value(DurableTaskInput {
        channel_id,
        trigger: trigger.clone(),
        action: step.action.clone(),
    })
    .map_err(|error| WorkflowError::InvalidDefinition(error.to_string()))?;
    let agent_pubkey = match &step.action {
        ActionDef::RunAgent { identity, .. } => Some(hex::decode(identity).map_err(|error| {
            WorkflowError::InvalidDefinition(format!(
                "step '{}': invalid agent identity: {error}",
                step.id
            ))
        })?),
        _ => None,
    };
    let output_schema = match &step.action {
        ActionDef::RunAgent { output_schema, .. } => Some(output_schema.clone()),
        ActionDef::VerifyArtifact { schema } => Some(schema.clone()),
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
mod tests {
    use super::*;

    fn trigger() -> TriggerContext {
        TriggerContext {
            channel_id: Uuid::nil().to_string(),
            webhook_fields: HashMap::from([("document_uri".into(), "file://case.pdf".into())]),
            ..TriggerContext::default()
        }
    }

    #[test]
    fn blueprint_is_deterministic_and_preserves_dependencies() {
        let run_id = Uuid::nil();
        let step = Step {
            id: "analysis".into(),
            name: None,
            if_expr: None,
            timeout_secs: None,
            depends_on: vec!["ingest".into()],
            action: ActionDef::RunAgent {
                agent: "helena".into(),
                identity: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".into(),
                prompt: "Analyze {{trigger.document_uri}}".into(),
                output_schema: json!({
                    "type": "object",
                    "required": ["decision"]
                }),
            },
        };
        let first =
            task_blueprint(run_id, Uuid::nil(), &trigger(), &step).expect("blueprint should build");
        let second =
            task_blueprint(run_id, Uuid::nil(), &trigger(), &step).expect("blueprint should build");
        assert_eq!(first.input, second.input);
        assert_eq!(first.depends_on, json!(["ingest"]));
        assert_eq!(first.idempotency_key, format!("{run_id}:analysis"));
        assert_eq!(
            first.output_schema,
            Some(json!({ "type": "object", "required": ["decision"] }))
        );
    }

    #[test]
    fn unsupported_coordinator_action_remains_explicit_in_input() {
        let step = Step {
            id: "publish".into(),
            name: None,
            if_expr: None,
            timeout_secs: None,
            depends_on: vec!["approval".into()],
            action: ActionDef::PublishArtifact {
                artifact: "decision".into(),
            },
        };
        let blueprint = task_blueprint(Uuid::nil(), Uuid::nil(), &trigger(), &step)
            .expect("blueprint should build");
        assert_eq!(blueprint.input["action"]["action"], "publish_artifact");
        assert_eq!(blueprint.input["action"]["artifact"], "decision");
    }
}
