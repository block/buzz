use buzz_core::job::JobResultPayload;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_event_id, parse_uuid, read_file_or_stdin, sdk_err};

fn parse_manifest(content: &str) -> Result<JobResultPayload, CliError> {
    let payload: JobResultPayload = serde_json::from_str(content)
        .map_err(|error| CliError::Usage(format!("invalid job handoff manifest: {error}")))?;
    payload
        .validate()
        .map_err(|error| CliError::Usage(error.to_string()))?;
    Ok(payload)
}

fn ensure_job_matches(payload: &JobResultPayload, job_event_id: &str) -> Result<(), CliError> {
    if payload.job_request.eq_ignore_ascii_case(job_event_id) {
        Ok(())
    } else {
        Err(CliError::Usage(
            "manifest jobRequest must match --job".into(),
        ))
    }
}

async fn cmd_handoff(
    client: &BuzzClient,
    channel: &str,
    job: &str,
    manifest: &str,
) -> Result<(), CliError> {
    let channel_id = parse_uuid(channel)?;
    let job_event_id = parse_event_id(job)?;
    let manifest_content = read_file_or_stdin(manifest)?;
    let payload = parse_manifest(&manifest_content)?;
    ensure_job_matches(&payload, &job_event_id.to_hex())?;

    let builder =
        buzz_sdk::build_job_result(channel_id, job_event_id, &payload).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let response = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&response));
    Ok(())
}

pub async fn dispatch(command: crate::JobsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match command {
        crate::JobsCmd::Handoff {
            channel,
            job,
            manifest,
        } => cmd_handoff(client, &channel, &job, &manifest).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOB_EVENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn valid_manifest() -> String {
        serde_json::json!({
            "schemaVersion": 1,
            "jobRequest": JOB_EVENT,
            "requestedOutcome": "Make the result inspectable",
            "outcome": "The handoff is ready.",
            "lastProgress": "Verification completed.",
            "disposition": "completed",
            "artifacts": [{
                "kind": "pull_request",
                "label": "Pull request",
                "reference": "https://github.com/block/buzz/pull/1",
                "sourceState": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }],
            "verification": [{
                "label": "just ci",
                "status": "passed",
                "evidence": "exit 0"
            }]
        })
        .to_string()
    }

    #[test]
    fn parses_and_validates_manifest() {
        let payload = parse_manifest(&valid_manifest()).expect("valid manifest");
        assert_eq!(payload.job_request, JOB_EVENT);
        assert_eq!(payload.artifacts.len(), 1);
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(matches!(
            parse_manifest("{not-json"),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn rejects_invalid_contract() {
        let manifest = valid_manifest().replace("\"schemaVersion\":1", "\"schemaVersion\":2");
        assert!(matches!(parse_manifest(&manifest), Err(CliError::Usage(_))));
    }

    #[test]
    fn rejects_manifest_for_different_job() {
        let payload = parse_manifest(&valid_manifest()).expect("valid manifest");
        assert!(matches!(
            ensure_job_matches(
                &payload,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            ),
            Err(CliError::Usage(_))
        ));
    }
}
