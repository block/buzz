use std::collections::VecDeque;
use std::path::Path;

use buzz_core::agent_job::{
    parse_agent_job_event, AgentJobCancel, AgentJobPayload, AgentJobRequest,
    AGENT_JOB_SCHEMA as JOB_SCHEMA, MAX_JOB_ARGV_ENTRIES as MAX_ARGV_ITEMS,
    MAX_JOB_ARGV_JSON_BYTES as MAX_ARGV_JSON_BYTES, MAX_JOB_ARG_BYTES as MAX_ARG_BYTES,
    MAX_JOB_CWD_BYTES as MAX_CWD_BYTES, MAX_JOB_DRIVER_BYTES as MAX_DRIVER_BYTES,
    MAX_JOB_REASON_BYTES as MAX_REASON_BYTES, MAX_JOB_SUMMARY_BYTES as MAX_SUMMARY_BYTES,
};
use chrono::{DateTime, Utc};
use nostr::{Event, EventId, PublicKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::sdk_err;
use crate::JobsCmd;

const JOB_STATES: [&str; 8] = [
    "requested",
    "accepted",
    "running",
    "cancelling",
    "succeeded",
    "failed",
    "cancelled",
    "lost",
];

#[derive(Debug, Serialize)]
struct JobStartOutput {
    job_id: Uuid,
    event_id: String,
    state: &'static str,
}

#[derive(Debug, Serialize)]
struct LocalJobLogsOutput {
    job_id: Uuid,
    local_only: bool,
    lines: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PublicJobLogsOutput {
    job_id: Uuid,
    raw_output: &'static str,
    public_summaries: Vec<PublicJobLogSummary>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PublicJobLogSummary {
    event_id: String,
    created_at: DateTime<Utc>,
    state: &'static str,
    attempt: u32,
    progress_seq: Option<u64>,
    summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RelayJobProjection {
    job_id: Uuid,
    request_event_id: String,
    channel_id: Uuid,
    requester_pubkey: String,
    target_pubkey: String,
    state: String,
    attempt: u32,
    progress_seq: Option<u64>,
    summary: String,
    cancel_requested: bool,
    terminal_event_id: Option<String>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RelayJobChainEntry {
    event_id: String,
    kind: u32,
    author_pubkey: String,
    attempt: Option<u32>,
    progress_seq: Option<u64>,
    created_at: DateTime<Utc>,
    event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RelayJobLookup {
    status: RelayJobProjection,
    chain: Vec<RelayJobChainEntry>,
}

pub async fn dispatch(cmd: JobsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        JobsCmd::Start {
            agent,
            channel,
            cwd,
            summary,
            driver,
            argv,
        } => cmd_start(client, &agent, channel, &driver, &argv, &cwd, &summary).await,
        JobsCmd::Status { job_id } => cmd_status(client, job_id).await,
        JobsCmd::List {
            agent,
            channel,
            state,
        } => cmd_list(client, agent.as_deref(), channel, state.as_deref()).await,
        JobsCmd::Stop { job_id, reason } => cmd_stop(client, job_id, &reason).await,
        JobsCmd::Logs { job_id, lines } => cmd_public_logs(client, job_id, lines).await,
    }
}

async fn cmd_start(
    client: &BuzzClient,
    agent: &str,
    channel: Uuid,
    driver: &str,
    argv: &[String],
    cwd: &str,
    summary: &str,
) -> Result<(), CliError> {
    validate_request(driver, argv, cwd, summary)?;
    let target = parse_pubkey(agent, "--agent")?;
    let job_id = Uuid::new_v4();
    let payload = AgentJobRequest {
        schema: JOB_SCHEMA,
        driver: driver.to_string(),
        argv: argv.to_vec(),
        cwd: cwd.to_string(),
        summary: summary.to_string(),
    };
    let builder = buzz_sdk::build_agent_job_request(channel, target, job_id, None, None, &payload)
        .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let output = JobStartOutput {
        job_id,
        event_id: event.id.to_hex(),
        state: "requested",
    };

    // Retries reuse this signed event: event-ID replay and job UUID idempotency stay distinct.
    match client.submit_event(event).await {
        Ok(_) => print_json(&output),
        Err(CliError::DeliveryUnknown(message)) => Err(CliError::DeliveryUnknown(format!(
            "job {} request event {}: {message}",
            output.job_id, output.event_id
        ))),
        Err(error) => Err(error),
    }
}

async fn load_job_lookup(client: &BuzzClient, job_id: Uuid) -> Result<RelayJobLookup, CliError> {
    match client.agent_job_status(job_id).await {
        Ok(body) => parse_indexed_response(&body),
        Err(CliError::Relay { status: 404, .. }) => Err(CliError::NotFound(format!(
            "job {job_id} was not found on the relay"
        ))),
        Err(error) => Err(error),
    }
}

async fn cmd_status(client: &BuzzClient, job_id: Uuid) -> Result<(), CliError> {
    print_json(&load_job_lookup(client, job_id).await?)
}

async fn cmd_list(
    client: &BuzzClient,
    agent: Option<&str>,
    channel: Option<Uuid>,
    state: Option<&str>,
) -> Result<(), CliError> {
    let target = agent
        .map(|value| parse_pubkey(value, "--agent").map(|pubkey| pubkey.to_hex()))
        .transpose()?;
    let state = state.map(validate_state).transpose()?;
    let jobs: Vec<RelayJobProjection> = parse_indexed_response(
        &client
            .agent_jobs_list(target.as_deref(), channel, state.as_deref(), 500)
            .await?,
    )?;
    print_json(&jobs)
}

async fn cmd_stop(client: &BuzzClient, job_id: Uuid, reason: &str) -> Result<(), CliError> {
    validate_len("--reason", reason, MAX_REASON_BYTES)?;
    let lookup = load_job_lookup(client, job_id).await?;
    let projection = lookup.status;
    if matches!(
        projection.state.as_str(),
        "succeeded" | "failed" | "cancelled" | "lost"
    ) {
        return Err(CliError::Usage(format!(
            "job {job_id} is already terminal ({})",
            projection.state
        )));
    }
    let target = PublicKey::parse(&projection.target_pubkey).map_err(|_| {
        CliError::Other(format!("relay returned an invalid target for job {job_id}"))
    })?;
    let request_event_id = EventId::parse(&projection.request_event_id).map_err(|_| {
        CliError::Other(format!(
            "relay returned an invalid request event for job {job_id}"
        ))
    })?;
    let payload = AgentJobCancel {
        schema: JOB_SCHEMA,
        job: job_id,
        reason: reason.to_string(),
    };
    let builder =
        buzz_sdk::build_agent_job_cancel(projection.channel_id, target, request_event_id, &payload)
            .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let output = serde_json::json!({
        "event_id": event.id.to_hex(),
        "job_id": job_id,
        "state": "cancelling"
    });
    client.submit_event(event).await?;
    print_json(&output)
}

pub(crate) async fn cmd_local_logs(
    runtime_receipt: &Path,
    job_id: Uuid,
    lines: u16,
) -> Result<(), CliError> {
    let client =
        buzz_runtime::RuntimeClient::from_receipt(runtime_receipt, buzz_runtime::Capability::Model)
            .await
            .map_err(local_control_error)?;
    let logs = client
        .jobs_logs(job_id, Some(lines))
        .await
        .map_err(local_control_error)?;
    print_json(&LocalJobLogsOutput {
        job_id: logs.job_id,
        local_only: logs.local_only,
        lines: logs.lines,
    })
}

async fn cmd_public_logs(client: &BuzzClient, job_id: Uuid, lines: u16) -> Result<(), CliError> {
    let lookup = load_job_lookup(client, job_id).await?;
    print_json(&PublicJobLogsOutput {
        job_id,
        raw_output: "local-only",
        public_summaries: public_log_summaries(&lookup, job_id, lines)?,
    })
}

fn public_log_summaries(
    lookup: &RelayJobLookup,
    job_id: Uuid,
    limit: u16,
) -> Result<Vec<PublicJobLogSummary>, CliError> {
    let limit = usize::from(limit.min(1_000));
    let mut summaries = VecDeque::with_capacity(limit);
    for entry in &lookup.chain {
        entry.event.verify().map_err(|error| {
            CliError::Other(format!(
                "relay returned an invalid signed event {} for job {job_id}: {error}",
                entry.event_id
            ))
        })?;
        if entry.event.id.to_hex() != entry.event_id {
            return Err(CliError::Other(format!(
                "relay returned a mismatched event ID for job {job_id}"
            )));
        }
        let parsed = parse_agent_job_event(&entry.event).map_err(|error| {
            CliError::Other(format!(
                "relay returned an invalid job event {}: {error}",
                entry.event_id
            ))
        })?;
        let created_at = DateTime::from_timestamp(entry.event.created_at.as_secs() as i64, 0)
            .ok_or_else(|| {
                CliError::Other(format!(
                    "relay returned an invalid event timestamp for job {job_id}"
                ))
            })?;
        if parsed.job != job_id {
            return Err(CliError::Other(format!(
                "relay returned event {} for a different job",
                entry.event_id
            )));
        }
        let summary = match parsed.payload {
            AgentJobPayload::Progress(payload) => Some(PublicJobLogSummary {
                event_id: entry.event_id.clone(),
                created_at,
                state: payload.state.as_str(),
                attempt: payload.attempt,
                progress_seq: Some(payload.seq),
                summary: payload.summary,
            }),
            AgentJobPayload::Result(payload) => Some(PublicJobLogSummary {
                event_id: entry.event_id.clone(),
                created_at,
                state: payload.state.as_str(),
                attempt: payload.attempt,
                progress_seq: None,
                summary: payload.summary,
            }),
            AgentJobPayload::Error(payload) => Some(PublicJobLogSummary {
                event_id: entry.event_id.clone(),
                created_at,
                state: payload.state.as_str(),
                attempt: payload.attempt,
                progress_seq: None,
                summary: payload.summary,
            }),
            AgentJobPayload::Request(_)
            | AgentJobPayload::Accepted(_)
            | AgentJobPayload::Cancel(_) => None,
        };
        if let Some(summary) = summary {
            if limit == 0 {
                continue;
            }
            if summaries.len() == limit {
                summaries.pop_front();
            }
            summaries.push_back(summary);
        }
    }
    Ok(summaries.into_iter().collect())
}

fn parse_indexed_response<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, CliError> {
    serde_json::from_str(body).map_err(|error| {
        CliError::Other(format!(
            "relay returned a malformed indexed job response: {error}"
        ))
    })
}

fn parse_pubkey(value: &str, flag: &str) -> Result<PublicKey, CliError> {
    PublicKey::parse(value)
        .map_err(|error| CliError::Usage(format!("{flag} must be a hex pubkey or npub: {error}")))
}

fn validate_request(
    driver: &str,
    argv: &[String],
    cwd: &str,
    summary: &str,
) -> Result<(), CliError> {
    validate_len("--driver", driver, MAX_DRIVER_BYTES)?;
    if driver != "lh" {
        return Err(CliError::Usage("schema 1 supports only --driver lh".into()));
    }
    if argv.is_empty() || argv.len() > MAX_ARGV_ITEMS {
        return Err(CliError::Usage(format!(
            "argv must contain between 1 and {MAX_ARGV_ITEMS} entries"
        )));
    }
    for (index, arg) in argv.iter().enumerate() {
        validate_len(&format!("argv[{index}]"), arg, MAX_ARG_BYTES)?;
    }
    let argv_json = serde_json::to_vec(argv)
        .map_err(|error| CliError::Usage(format!("argv cannot be serialized: {error}")))?;
    if argv_json.len() > MAX_ARGV_JSON_BYTES {
        return Err(CliError::Usage(format!(
            "argv JSON exceeds {MAX_ARGV_JSON_BYTES} bytes"
        )));
    }
    validate_len("--cwd", cwd, MAX_CWD_BYTES)?;
    if !Path::new(cwd).is_absolute() {
        return Err(CliError::Usage("--cwd must be an absolute path".into()));
    }
    validate_len("--summary", summary, MAX_SUMMARY_BYTES)
}

fn validate_state(value: &str) -> Result<String, CliError> {
    if JOB_STATES.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(CliError::Usage(format!(
            "--state must be one of: {}",
            JOB_STATES.join(", ")
        )))
    }
}

fn validate_len(name: &str, value: &str, max: usize) -> Result<(), CliError> {
    if value.len() > max {
        return Err(CliError::Usage(format!("{name} exceeds {max} UTF-8 bytes")));
    }
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let json = serde_json::to_string(value)
        .map_err(|error| CliError::Other(format!("failed to serialize output: {error}")))?;
    println!("{json}");
    Ok(())
}

fn local_control_error(error: impl std::fmt::Display) -> CliError {
    CliError::Other(format!("local runtime control failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    const TARGET: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn tag_value(event: &Event, name: &str) -> Option<String> {
        let values: Vec<_> = event
            .tags
            .iter()
            .filter_map(|tag| {
                let values = tag.as_slice();
                (values.first().map(String::as_str) == Some(name))
                    .then(|| values.get(1).cloned())
                    .flatten()
            })
            .collect();
        (values.len() == 1).then(|| values[0].clone())
    }

    #[test]
    fn request_bounds_reject_before_signing() {
        let too_many = vec!["x".to_string(); MAX_ARGV_ITEMS + 1];
        assert!(validate_request("lh", &too_many, "/workspace", "ok").is_err());
        assert!(
            validate_request("lh", &["x".repeat(MAX_ARG_BYTES + 1)], "/workspace", "ok").is_err()
        );
        assert!(validate_request("lh", &["ok".into()], "relative", "ok").is_err());
        assert!(validate_request("shell", &["ok".into()], "/workspace", "ok").is_err());
        assert!(validate_len(
            "--reason",
            &"x".repeat(MAX_REASON_BYTES + 1),
            MAX_REASON_BYTES
        )
        .is_err());
    }

    #[test]
    fn signed_cancel_still_targets_canonical_request_agent() {
        let requester = Keys::generate();
        let target = PublicKey::parse(TARGET).unwrap();
        let job_id = Uuid::parse_str("12345678-1234-4234-8234-123456789abc").unwrap();
        let request_event_id = EventId::parse(&"1".repeat(64)).unwrap();
        let payload = AgentJobCancel {
            schema: JOB_SCHEMA,
            job: job_id,
            reason: "Stop requested".into(),
        };
        let event =
            buzz_sdk::build_agent_job_cancel(Uuid::nil(), target, request_event_id, &payload)
                .unwrap()
                .sign_with_keys(&requester)
                .unwrap();
        assert_eq!(event.pubkey, requester.public_key());
        assert_eq!(tag_value(&event, "p").as_deref(), Some(TARGET));
        assert_eq!(tag_value(&event, "job"), Some(job_id.to_string()));
    }

    #[test]
    fn indexed_status_and_list_use_canonical_projection_shape() {
        let signed = nostr::EventBuilder::text_note("request")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let projection = RelayJobProjection {
            job_id: Uuid::parse_str("12345678-1234-4234-8234-123456789abc").unwrap(),
            request_event_id: signed.id.to_hex(),
            channel_id: Uuid::parse_str("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").unwrap(),
            requester_pubkey: "c".repeat(64),
            target_pubkey: "d".repeat(64),
            state: "running".into(),
            attempt: 1,
            progress_seq: Some(2),
            summary: "Working".into(),
            cancel_requested: false,
            terminal_event_id: None,
            updated_at: DateTime::parse_from_rfc3339("2026-08-02T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let lookup = RelayJobLookup {
            status: projection.clone(),
            chain: vec![RelayJobChainEntry {
                event_id: signed.id.to_hex(),
                kind: signed.kind.as_u16() as u32,
                author_pubkey: signed.pubkey.to_hex(),
                attempt: None,
                progress_seq: None,
                created_at: DateTime::from_timestamp(signed.created_at.as_secs() as i64, 0)
                    .unwrap(),
                event: signed,
            }],
        };
        let status: RelayJobLookup =
            parse_indexed_response(&serde_json::to_string(&lookup).unwrap()).unwrap();
        let list: Vec<RelayJobProjection> =
            parse_indexed_response(&serde_json::to_string(&vec![projection.clone()]).unwrap())
                .unwrap();
        assert_eq!(status.status, projection);
        status.chain[0]
            .event
            .verify()
            .expect("status chain preserves a signed event");
        assert_eq!(status.chain.len(), 1);
        assert_eq!(list, vec![projection]);
    }

    #[test]
    fn public_logs_are_bounded_signed_summaries_and_not_raw_output() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let job_id = Uuid::parse_str("12345678-1234-4234-8234-123456789abc").unwrap();
        let channel_id = Uuid::parse_str("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").unwrap();
        let request_event_id = EventId::parse(&"1".repeat(64)).unwrap();
        let mut chain = Vec::new();
        for seq in 1..=3 {
            let payload = buzz_core::agent_job::AgentJobProgress {
                schema: JOB_SCHEMA,
                job: job_id,
                attempt: 1,
                seq,
                state: buzz_core::agent_job::AgentJobProgressState::Running,
                summary: format!("public progress {seq}"),
                artifacts: Vec::new(),
            };
            let event = buzz_sdk::build_agent_job_progress(
                channel_id,
                requester.public_key(),
                request_event_id,
                &payload,
            )
            .unwrap()
            .sign_with_keys(&agent)
            .unwrap();
            chain.push(RelayJobChainEntry {
                event_id: event.id.to_hex(),
                kind: event.kind.as_u16() as u32,
                author_pubkey: event.pubkey.to_hex(),
                attempt: Some(1),
                progress_seq: Some(seq),
                created_at: DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).unwrap(),
                event,
            });
        }
        let lookup = RelayJobLookup {
            status: RelayJobProjection {
                job_id,
                request_event_id: request_event_id.to_hex(),
                channel_id,
                requester_pubkey: requester.public_key().to_hex(),
                target_pubkey: agent.public_key().to_hex(),
                state: "running".into(),
                attempt: 1,
                progress_seq: Some(3),
                summary: "public progress 3".into(),
                cancel_requested: false,
                terminal_event_id: None,
                updated_at: Utc::now(),
            },
            chain,
        };

        let summaries = public_log_summaries(&lookup, job_id, 2).unwrap();
        assert_eq!(
            summaries
                .iter()
                .map(|entry| entry.summary.as_str())
                .collect::<Vec<_>>(),
            ["public progress 2", "public progress 3"]
        );
        let output = PublicJobLogsOutput {
            job_id,
            raw_output: "local-only",
            public_summaries: summaries,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"raw_output\":\"local-only\""));
        assert!(!json.contains("\"lines\""));
    }

    #[test]
    fn start_output_is_stable_and_does_not_echo_job_spec_or_secrets() {
        let output = JobStartOutput {
            job_id: Uuid::parse_str("12345678-1234-4234-8234-123456789abc").unwrap(),
            event_id: "f".repeat(64),
            state: "requested",
        };
        let json = serde_json::to_string(&output).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\"job_id\":\"12345678-1234-4234-8234-123456789abc\",\"event_id\":\"{}\",\"state\":\"requested\"}}",
                "f".repeat(64)
            )
        );
        assert!(!json.contains("BUZZ_PRIVATE_KEY"));
        assert!(!json.contains("controlToken"));
    }
}
