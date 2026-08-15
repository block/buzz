use buzz_core::kind::{KIND_TASK_REQUESTED, KIND_TASK_RESOLVED, KIND_TASK_UPDATED};
use buzz_core::task::{
    TaskEventV1, TaskPriority, TaskRequestedV1, TaskResolution, TaskResolvedV1, TaskType,
    TaskUpdatedV1,
};
use buzz_sdk::TaskRef;
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use uuid::Uuid;

use crate::client::{
    normalize_events, normalize_write_response, print_create_response, BuzzClient,
};
use crate::error::CliError;
use crate::validate::{parse_event_id, parse_uuid, sdk_err};

fn parse_task_type(value: &str) -> Result<TaskType, CliError> {
    match value {
        "reply" => Ok(TaskType::Reply),
        "approval" => Ok(TaskType::Approval),
        "choice" => Ok(TaskType::Choice),
        "review" => Ok(TaskType::Review),
        other => Err(CliError::Usage(format!(
            "--type must be reply, approval, choice, or review (got '{other}')"
        ))),
    }
}

fn parse_priority(value: &str) -> Result<TaskPriority, CliError> {
    match value {
        "low" => Ok(TaskPriority::Low),
        "medium" => Ok(TaskPriority::Medium),
        "high" => Ok(TaskPriority::High),
        other => Err(CliError::Usage(format!(
            "--priority must be low, medium, or high (got '{other}')"
        ))),
    }
}

fn parse_resolution(value: &str) -> Result<TaskResolution, CliError> {
    match value {
        "resolved" => Ok(TaskResolution::Resolved),
        "withdrawn" => Ok(TaskResolution::Withdrawn),
        other => Err(CliError::Usage(format!(
            "--resolution must be resolved or withdrawn (got '{other}')"
        ))),
    }
}

fn parse_due(value: Option<&str>) -> Result<Option<DateTime<Utc>>, CliError> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|e| CliError::Usage(format!("--due must be RFC 3339: {e}")))
        })
        .transpose()
}

/// Resolve the owner `p` tag: an explicit flag wins, otherwise the owner
/// carried by the NIP-OA auth tag the ACP harness injects.
fn resolve_owner_pubkey(client: &BuzzClient, owner: Option<&str>) -> Result<PublicKey, CliError> {
    let value = match owner {
        Some(value) => value.to_string(),
        None => client.auth_tag_owner_hex().ok_or_else(|| {
            CliError::Usage("--owner is required when BUZZ_AUTH_TAG carries no owner pubkey".into())
        })?,
    };
    PublicKey::parse(&value).map_err(|e| CliError::Usage(format!("invalid owner pubkey: {e}")))
}

/// Fetch the agent's own profile display name for the `agentName` snapshot.
async fn own_agent_name(client: &BuzzClient) -> Result<String, CliError> {
    let me = client.keys().public_key().to_hex();
    let filter = serde_json::json!({"kinds": [0], "authors": [me], "limit": 1});
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse profile query: {e}")))?;
    events
        .first()
        .and_then(|event| event.get("content"))
        .and_then(|content| content.as_str())
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .and_then(|profile| {
            ["display_name", "name"].iter().find_map(|field| {
                profile
                    .get(field)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
        })
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| CliError::Usage("no profile display name found; pass --agent-name".into()))
}

/// Build the signed identity every event for one task must repeat exactly.
///
/// Task reads are owner-gated on the relay, so the authoring agent cannot
/// recover this identity by querying its own request back — the caller must
/// supply the original request's channel, source, and owner on every
/// transition.
fn task_ref(
    client: &BuzzClient,
    task_id: Uuid,
    channel: &str,
    source: &str,
    owner: Option<&str>,
) -> Result<TaskRef, CliError> {
    Ok(TaskRef {
        task_id,
        owner_pubkey: resolve_owner_pubkey(client, owner)?,
        agent_pubkey: client.keys().public_key(),
        channel_id: parse_uuid(channel)?,
        source_event_id: parse_event_id(source)?,
    })
}

/// Sign, re-validate against the Buzz Tasks contract, and submit.
///
/// The parse round-trip runs the exact validation the relay applies on
/// ingest, so malformed input fails locally with the contract's error.
async fn sign_check_submit(
    client: &BuzzClient,
    builder: nostr::EventBuilder,
) -> Result<String, CliError> {
    let event = client.sign_event(builder)?;
    TaskEventV1::parse(&event)
        .map_err(|e| CliError::Usage(format!("task event failed the Buzz Tasks contract: {e}")))?;
    client.submit_event(event).await
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_request(
    client: &BuzzClient,
    channel: &str,
    source: &str,
    owner: Option<&str>,
    task_type: &str,
    title: String,
    context: Option<String>,
    priority: &str,
    due: Option<&str>,
    agent_name: Option<String>,
    task_id: Option<&str>,
) -> Result<(), CliError> {
    let task_id = task_id
        .map(parse_uuid)
        .transpose()?
        .unwrap_or_else(Uuid::new_v4);
    let task = task_ref(client, task_id, channel, source, owner)?;
    let agent_name = match agent_name {
        Some(name) => name,
        None => own_agent_name(client).await?,
    };
    let payload = TaskRequestedV1 {
        task_type: parse_task_type(task_type)?,
        title,
        context,
        priority: parse_priority(priority)?,
        due_at: parse_due(due)?,
        agent_name,
        source_version: 1,
        source_updated_at: Utc::now(),
    };
    let builder = buzz_sdk::build_task_requested(&task, &payload).map_err(sdk_err)?;
    let resp = sign_check_submit(client, builder).await?;
    print_create_response(&resp, "task_id", &task_id.to_string());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_update(
    client: &BuzzClient,
    task: &str,
    channel: &str,
    source: &str,
    owner: Option<&str>,
    version: i64,
    task_type: &str,
    title: String,
    context: Option<String>,
    priority: &str,
    due: Option<&str>,
    agent_name: Option<String>,
) -> Result<(), CliError> {
    let task = task_ref(client, parse_uuid(task)?, channel, source, owner)?;
    let agent_name = match agent_name {
        Some(name) => name,
        None => own_agent_name(client).await?,
    };
    let payload = TaskUpdatedV1 {
        task_type: parse_task_type(task_type)?,
        title,
        context,
        priority: parse_priority(priority)?,
        due_at: parse_due(due)?,
        agent_name,
        source_version: version,
        source_updated_at: Utc::now(),
    };
    let builder = buzz_sdk::build_task_updated(&task, &payload).map_err(sdk_err)?;
    let resp = sign_check_submit(client, builder).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_resolve(
    client: &BuzzClient,
    task: &str,
    channel: &str,
    source: &str,
    owner: Option<&str>,
    version: i64,
    resolution: &str,
) -> Result<(), CliError> {
    let task = task_ref(client, parse_uuid(task)?, channel, source, owner)?;
    let payload = TaskResolvedV1 {
        resolution: parse_resolution(resolution)?,
        source_version: version,
        source_updated_at: Utc::now(),
    };
    let builder = buzz_sdk::build_task_resolved(&task, &payload).map_err(sdk_err)?;
    let resp = sign_check_submit(client, builder).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// List task events addressed to the caller as owner.
///
/// Task kinds are `#p`-gated on the relay, so this read only returns events
/// whose owner `p` tag equals the caller — the owner-side view. Authoring
/// agents keep task identity and versions in their own source state.
pub async fn cmd_list(
    client: &BuzzClient,
    task: Option<&str>,
    channel: Option<&str>,
    limit: Option<u64>,
) -> Result<(), CliError> {
    let me = client.keys().public_key().to_hex();
    let mut filter = serde_json::json!({
        "kinds": [KIND_TASK_REQUESTED, KIND_TASK_UPDATED, KIND_TASK_RESOLVED],
        "#p": [me],
        "limit": limit.unwrap_or(100),
    });
    if let Some(task) = task {
        filter["#d"] = serde_json::json!([parse_uuid(task)?.to_string()]);
    }
    if let Some(channel) = channel {
        filter["#h"] = serde_json::json!([parse_uuid(channel)?.to_string()]);
    }
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse task list: {e}")))?;
    println!("{}", normalize_events(&events));
    Ok(())
}

pub async fn dispatch(cmd: crate::TasksCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::TasksCmd;
    match cmd {
        TasksCmd::Request {
            channel,
            source,
            owner,
            task_type,
            title,
            context,
            priority,
            due,
            agent_name,
            task_id,
        } => {
            cmd_request(
                client,
                &channel,
                &source,
                owner.as_deref(),
                &task_type,
                title,
                context,
                &priority,
                due.as_deref(),
                agent_name,
                task_id.as_deref(),
            )
            .await
        }
        TasksCmd::Update {
            task,
            channel,
            source,
            owner,
            version,
            task_type,
            title,
            context,
            priority,
            due,
            agent_name,
        } => {
            cmd_update(
                client,
                &task,
                &channel,
                &source,
                owner.as_deref(),
                version,
                &task_type,
                title,
                context,
                &priority,
                due.as_deref(),
                agent_name,
            )
            .await
        }
        TasksCmd::Resolve {
            task,
            channel,
            source,
            owner,
            version,
            resolution,
        } => {
            cmd_resolve(
                client,
                &task,
                &channel,
                &source,
                owner.as_deref(),
                version,
                &resolution,
            )
            .await
        }
        TasksCmd::List {
            task,
            channel,
            limit,
        } => cmd_list(client, task.as_deref(), channel.as_deref(), limit).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn payload_field_parsers_accept_contract_values_and_reject_others() {
        assert_eq!(parse_task_type("review").unwrap(), TaskType::Review);
        assert_eq!(parse_task_type("reply").unwrap(), TaskType::Reply);
        assert_eq!(parse_task_type("approval").unwrap(), TaskType::Approval);
        assert_eq!(parse_task_type("choice").unwrap(), TaskType::Choice);
        assert!(parse_task_type("urgent").is_err());

        assert_eq!(parse_priority("low").unwrap(), TaskPriority::Low);
        assert_eq!(parse_priority("medium").unwrap(), TaskPriority::Medium);
        assert_eq!(parse_priority("high").unwrap(), TaskPriority::High);
        assert!(parse_priority("critical").is_err());

        assert_eq!(
            parse_resolution("resolved").unwrap(),
            TaskResolution::Resolved
        );
        assert_eq!(
            parse_resolution("withdrawn").unwrap(),
            TaskResolution::Withdrawn
        );
        assert!(parse_resolution("done").is_err());

        assert_eq!(parse_due(None).unwrap(), None);
        assert_eq!(
            parse_due(Some("2026-08-15T12:00:00Z")).unwrap(),
            Some(chrono::Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap())
        );
        assert!(parse_due(Some("tomorrow")).is_err());
    }
}
