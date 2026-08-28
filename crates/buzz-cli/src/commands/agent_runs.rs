//! Agent-facing CLI for durable multi-agent workflow run events.

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, sdk_err};

const RUN_SNAPSHOT_KIND: u32 = buzz_core::kind::KIND_AGENT_WORKFLOW_RUN;
const RUN_RECEIPT_KINDS: [u32; 4] = [
    buzz_core::kind::KIND_AGENT_WORKFLOW_TASK,
    buzz_core::kind::KIND_AGENT_WORKFLOW_CHECKPOINT,
    buzz_core::kind::KIND_AGENT_WORKFLOW_ARTIFACT,
    buzz_core::kind::KIND_AGENT_WORKFLOW_TRANSITION,
];

#[derive(Clone, Copy)]
enum ReceiptKind {
    Task,
    Checkpoint,
    Artifact,
    Transition,
}

/// Read the latest durable run snapshot without steering an agent.
async fn status(client: &BuzzClient, run: &str) -> Result<(), CliError> {
    let run = parse_uuid(run)?;
    let filter = status_filter(run);
    let response = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&response)
        .map_err(|error| CliError::Other(format!("invalid relay response: {error}")))?;
    match events.first() {
        Some(event) => println!("{event}"),
        None => println!("null"),
    }
    Ok(())
}

/// Read append-only task, checkpoint, artifact, and transition receipts.
async fn history(client: &BuzzClient, run: &str, limit: Option<u32>) -> Result<(), CliError> {
    let run = parse_uuid(run)?;
    let filter = history_filter(run, limit);
    let response = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&response)
        .map_err(|error| CliError::Other(format!("invalid relay response: {error}")))?;
    println!(
        "{}",
        serde_json::to_string(&events)
            .map_err(|error| CliError::Other(format!("response serialization failed: {error}")))?
    );
    Ok(())
}

fn status_filter(run: uuid::Uuid) -> serde_json::Value {
    serde_json::json!({
        "kinds": [RUN_SNAPSHOT_KIND],
        "#d": [run.to_string()],
        "limit": 1
    })
}

fn history_filter(run: uuid::Uuid, limit: Option<u32>) -> serde_json::Value {
    serde_json::json!({
        "kinds": RUN_RECEIPT_KINDS,
        "#d": [run.to_string()],
        "limit": limit.unwrap_or(100).clamp(1, 1_000)
    })
}

/// Publish the latest channel-visible durable run snapshot.
async fn publish_snapshot(
    client: &BuzzClient,
    coordinates: RunCoordinates<'_>,
    participants: &[String],
    content: &str,
) -> Result<(), CliError> {
    let channel = parse_uuid(coordinates.channel)?;
    let workflow = parse_uuid(coordinates.workflow)?;
    let run = parse_uuid(coordinates.run)?;
    let content = read_or_stdin(content)?;
    let participants = participant_refs(participants);
    let builder =
        buzz_sdk::build_agent_workflow_run(channel, workflow, run, &participants, &content)
            .map_err(sdk_err)?;
    publish(client, builder).await
}

/// Publish an append-only durable run receipt signed by the current identity.
async fn publish_receipt(
    client: &BuzzClient,
    kind: ReceiptKind,
    coordinates: RunCoordinates<'_>,
    task: Option<&str>,
    participants: &[String],
    content: &str,
) -> Result<(), CliError> {
    let channel = parse_uuid(coordinates.channel)?;
    let workflow = parse_uuid(coordinates.workflow)?;
    let run = parse_uuid(coordinates.run)?;
    let content = read_or_stdin(content)?;
    let participants = participant_refs(participants);
    let builder = match kind {
        ReceiptKind::Task => buzz_sdk::build_agent_workflow_task(
            channel,
            workflow,
            run,
            required_task(task)?,
            &participants,
            &content,
        ),
        ReceiptKind::Checkpoint => buzz_sdk::build_agent_workflow_checkpoint(
            channel,
            workflow,
            run,
            required_task(task)?,
            &participants,
            &content,
        ),
        ReceiptKind::Artifact => buzz_sdk::build_agent_workflow_artifact(
            channel,
            workflow,
            run,
            required_task(task)?,
            &participants,
            &content,
        ),
        ReceiptKind::Transition => buzz_sdk::build_agent_workflow_transition(
            channel,
            workflow,
            run,
            &participants,
            &content,
        ),
    }
    .map_err(sdk_err)?;
    publish(client, builder).await
}

async fn publish(client: &BuzzClient, builder: nostr::EventBuilder) -> Result<(), CliError> {
    let event = client.sign_event(builder)?;
    let response = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&response));
    Ok(())
}

fn required_task(task: Option<&str>) -> Result<uuid::Uuid, CliError> {
    let task = task.ok_or_else(|| CliError::Usage("--task is required".into()))?;
    parse_uuid(task)
}

fn participant_refs(participants: &[String]) -> Vec<&str> {
    participants.iter().map(String::as_str).collect()
}

/// Shared run coordinates supplied to receipt builders.
struct RunCoordinates<'a> {
    /// Channel UUID.
    pub channel: &'a str,
    /// Workflow UUID.
    pub workflow: &'a str,
    /// Run UUID.
    pub run: &'a str,
}

/// Dispatch a durable agent-run CLI operation.
pub(crate) async fn dispatch(
    cmd: crate::AgentRunsCmd,
    client: &BuzzClient,
) -> Result<(), CliError> {
    use crate::AgentRunsCmd;
    match cmd {
        AgentRunsCmd::Status { run } => status(client, &run).await,
        AgentRunsCmd::History { run, limit } => history(client, &run, limit).await,
        AgentRunsCmd::Snapshot {
            channel,
            workflow,
            run,
            participant,
            content,
        } => {
            publish_snapshot(
                client,
                RunCoordinates {
                    channel: &channel,
                    workflow: &workflow,
                    run: &run,
                },
                &participant,
                &content,
            )
            .await
        }
        AgentRunsCmd::Task {
            channel,
            workflow,
            run,
            task,
            participant,
            content,
        } => {
            publish_cli_receipt(
                client,
                ReceiptKind::Task,
                &channel,
                &workflow,
                &run,
                Some(&task),
                &participant,
                &content,
            )
            .await
        }
        AgentRunsCmd::Checkpoint {
            channel,
            workflow,
            run,
            task,
            participant,
            content,
        } => {
            publish_cli_receipt(
                client,
                ReceiptKind::Checkpoint,
                &channel,
                &workflow,
                &run,
                Some(&task),
                &participant,
                &content,
            )
            .await
        }
        AgentRunsCmd::Artifact {
            channel,
            workflow,
            run,
            task,
            participant,
            content,
        } => {
            publish_cli_receipt(
                client,
                ReceiptKind::Artifact,
                &channel,
                &workflow,
                &run,
                Some(&task),
                &participant,
                &content,
            )
            .await
        }
        AgentRunsCmd::Transition {
            channel,
            workflow,
            run,
            participant,
            content,
        } => {
            publish_cli_receipt(
                client,
                ReceiptKind::Transition,
                &channel,
                &workflow,
                &run,
                None,
                &participant,
                &content,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn publish_cli_receipt(
    client: &BuzzClient,
    kind: ReceiptKind,
    channel: &str,
    workflow: &str,
    run: &str,
    task: Option<&str>,
    participants: &[String],
    content: &str,
) -> Result<(), CliError> {
    publish_receipt(
        client,
        kind,
        RunCoordinates {
            channel,
            workflow,
            run,
        },
        task,
        participants,
        content,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_filters_use_interoperable_d_coordinate() {
        let run = uuid::Uuid::new_v4();
        for filter in [status_filter(run), history_filter(run, Some(5_000))] {
            assert_eq!(filter["#d"], serde_json::json!([run.to_string()]));
            assert!(filter.get("#run").is_none());
        }
        assert_eq!(history_filter(run, Some(5_000))["limit"], 1_000);
    }
}
