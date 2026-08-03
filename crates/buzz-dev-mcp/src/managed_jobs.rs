use crate::collaboration::CollaborationClient;
use buzz_runtime::{
    protocol::{
        MAX_ARGV_ELEMENTS, MAX_ARG_BYTES, MAX_CWD_BYTES, MAX_JOB_ARGV_JSON_BYTES,
        MAX_LOG_TAIL_LINES, MAX_SUMMARY_BYTES,
    },
    JobId, JobStartRequest, JobState, RuntimeClient,
};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobsStartParams {
    /// Channel UUID that owns the durable job.
    pub channel_id: String,
    /// Triggering Buzz event ID, when the job was requested from a message.
    #[serde(default)]
    pub source_event_id: Option<String>,
    /// Optional target managed agent. Omit or use the current identity for a local durable job.
    #[serde(default)]
    pub target_agent: Option<String>,
    /// Legacy Harness arguments. The executable and driver are fixed by the privileged runtime.
    pub argv: Vec<String>,
    /// Absolute workspace directory approved by the runtime operator.
    pub cwd: String,
    /// Human-readable purpose, at most 4 KiB.
    pub summary: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobIdParams {
    /// Durable job UUID.
    pub job_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobsLogsParams {
    /// Durable job UUID.
    pub job_id: String,
    /// Number of trailing lines. Defaults to 100 and is capped at 1,000.
    #[serde(default)]
    pub tail_lines: Option<u16>,
}

fn parse_job_id(value: &str) -> Result<JobId, ErrorData> {
    JobId::parse_str(value).map_err(|_| ErrorData::invalid_params("job_id must be a UUID", None))
}

fn parse_channel_id(value: &str) -> Result<Uuid, ErrorData> {
    Uuid::parse_str(value).map_err(|_| ErrorData::invalid_params("channel_id must be a UUID", None))
}

fn validate_event_id(value: &str) -> Result<(), ErrorData> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ErrorData::invalid_params(
            "source_event_id must be 64 lowercase hex characters",
            None,
        ));
    }
    Ok(())
}

fn serialize<T: serde::Serialize>(value: &T) -> Result<String, ErrorData> {
    serde_json::to_string(value)
        .map_err(|_| ErrorData::internal_error("cannot encode runtime response", None))
}

fn runtime_error() -> ErrorData {
    ErrorData::internal_error("managed runtime request failed", None)
}

fn validate_start(params: &JobsStartParams) -> Result<Uuid, ErrorData> {
    let channel_id = parse_channel_id(&params.channel_id)?;
    if channel_id.is_nil() {
        return Err(ErrorData::invalid_params(
            "channel_id must not be nil",
            None,
        ));
    }
    if let Some(source_event_id) = &params.source_event_id {
        validate_event_id(source_event_id)?;
    }
    if params.argv.len() > MAX_ARGV_ELEMENTS {
        return Err(ErrorData::invalid_params(
            "argv contains more than 256 elements",
            None,
        ));
    }
    if params.argv.iter().any(|arg| arg.len() > MAX_ARG_BYTES) {
        return Err(ErrorData::invalid_params(
            "an argv element exceeds 8 KiB",
            None,
        ));
    }
    let argv_json = serde_json::to_vec(&params.argv)
        .map_err(|_| ErrorData::invalid_params("argv is not encodable", None))?;
    if argv_json.len() > MAX_JOB_ARGV_JSON_BYTES {
        return Err(ErrorData::invalid_params(
            "encoded argv exceeds 64 KiB",
            None,
        ));
    }
    if params.cwd.is_empty() || params.cwd.len() > MAX_CWD_BYTES {
        return Err(ErrorData::invalid_params(
            "cwd must be non-empty and at most 4 KiB",
            None,
        ));
    }
    if !std::path::Path::new(&params.cwd).is_absolute() {
        return Err(ErrorData::invalid_params("cwd must be absolute", None));
    }
    if params.summary.is_empty() || params.summary.len() > MAX_SUMMARY_BYTES {
        return Err(ErrorData::invalid_params(
            "summary must be non-empty and at most 4 KiB",
            None,
        ));
    }
    Ok(channel_id)
}

pub(crate) async fn jobs_start(
    client: &RuntimeClient,
    collaboration: &CollaborationClient,
    params: JobsStartParams,
) -> Result<String, ErrorData> {
    let channel_id = validate_start(&params)?;
    let target = params
        .target_agent
        .as_deref()
        .map(nostr::PublicKey::parse)
        .transpose()
        .map_err(|_| ErrorData::invalid_params("target_agent must be a pubkey or npub", None))?;
    if let Some(target) =
        target.filter(|target| target.to_hex() != collaboration.current_pubkey().to_hex())
    {
        let source_event_id = params
            .source_event_id
            .as_deref()
            .map(nostr::EventId::from_hex)
            .transpose()
            .map_err(|_| ErrorData::invalid_params("source_event_id is invalid", None))?;
        return collaboration
            .jobs_request_remote(
                channel_id,
                target,
                source_event_id,
                params.argv,
                params.cwd,
                params.summary,
            )
            .await;
    }
    let status = client
        .jobs_start(JobStartRequest {
            channel_id,
            source_event_id: params.source_event_id,
            driver: "lh".to_owned(),
            argv: params.argv,
            cwd: params.cwd,
            summary: params.summary,
        })
        .await
        .map_err(|_| runtime_error())?;
    if !matches!(status.state, JobState::Accepted | JobState::Running) {
        return Err(ErrorData::internal_error(
            "managed runtime did not accept the job",
            None,
        ));
    }
    serialize(&serde_json::json!({
        "job_id": status.job_id,
        "state": "accepted",
    }))
}

pub(crate) async fn jobs_status(
    client: &RuntimeClient,
    params: JobIdParams,
) -> Result<String, ErrorData> {
    let status = client
        .jobs_status(parse_job_id(&params.job_id)?)
        .await
        .map_err(|_| runtime_error())?;
    serialize(&status)
}

pub(crate) async fn jobs_logs(
    client: &RuntimeClient,
    params: JobsLogsParams,
) -> Result<String, ErrorData> {
    let lines = params.tail_lines.map(|lines| lines.min(MAX_LOG_TAIL_LINES));
    let logs = client
        .jobs_logs(parse_job_id(&params.job_id)?, lines)
        .await
        .map_err(|_| runtime_error())?;
    serialize(&logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_params() -> JobsStartParams {
        JobsStartParams {
            channel_id: Uuid::from_u128(1).to_string(),
            source_event_id: Some("a".repeat(64)),
            target_agent: None,
            argv: vec![],
            cwd: std::env::current_dir().unwrap().display().to_string(),
            summary: "safe".into(),
        }
    }

    #[test]
    fn start_schema_has_no_privileged_fields() {
        let schema = schemars::schema_for!(JobsStartParams);
        let json = serde_json::to_value(schema).unwrap();
        let properties = json["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 6);
        for required in [
            "channel_id",
            "source_event_id",
            "target_agent",
            "argv",
            "cwd",
            "summary",
        ] {
            assert!(properties.contains_key(required));
        }
        for denied in ["driver", "executable", "env", "stdin", "shell"] {
            assert!(!properties.contains_key(denied));
        }
    }

    #[test]
    fn rejects_unknown_privileged_fields() {
        for field in ["driver", "executable", "env", "stdin", "shell"] {
            let input = format!(
                r#"{{"channel_id":"{}","argv":[],"cwd":"/tmp","summary":"safe","{field}":"denied"}}"#,
                Uuid::nil()
            );
            assert!(serde_json::from_str::<JobsStartParams>(&input).is_err());
        }
    }

    #[test]
    fn enforces_wire_limits_before_runtime_call() {
        let mut params = valid_params();
        params.argv = vec![String::new(); MAX_ARGV_ELEMENTS + 1];
        assert!(validate_start(&params).is_err());

        let mut params = valid_params();
        params.argv = vec!["x".repeat(MAX_ARG_BYTES + 1)];
        assert!(validate_start(&params).is_err());

        let mut params = valid_params();
        params.cwd = "x".repeat(MAX_CWD_BYTES + 1);
        assert!(validate_start(&params).is_err());

        let mut params = valid_params();
        params.summary = "x".repeat(MAX_SUMMARY_BYTES + 1);
        assert!(validate_start(&params).is_err());

        let mut params = valid_params();
        params.source_event_id = Some("A".repeat(64));
        assert!(validate_start(&params).is_err());

        let mut params = valid_params();
        params.channel_id = Uuid::nil().to_string();
        assert!(validate_start(&params).is_err());

        let mut params = valid_params();
        params.cwd.clear();
        assert!(validate_start(&params).is_err());

        let mut params = valid_params();
        params.summary.clear();
        assert!(validate_start(&params).is_err());
    }
}
