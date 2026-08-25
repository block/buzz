//! Local CML validation, canonicalization, and signed lifecycle commands.

use std::{fs, io::Read};

use crate::error::CliError;
use buzz_core::cml_event::{CmlRole, CmlTransition};
use nostr::{Event, EventId};

/// Validate CML text without performing I/O.
pub fn validate_input(input: &str) -> Result<(), CliError> {
    buzz_core::cml::parse_cml(input)
        .map(|_| ())
        .map_err(|error| CliError::Usage(error.to_string()))
}

/// Parse and return canonical CML without performing I/O.
pub fn canonicalize_input(input: &str) -> Result<String, CliError> {
    buzz_core::cml::parse_cml(input)
        .and_then(|task| task.to_canonical_json())
        .map_err(|error| CliError::Usage(error.to_string()))
}

/// Run `buzz cml validate <path|->` locally.
pub fn cmd_validate(path: &str) -> Result<(), CliError> {
    let input = read_input(path)?;
    validate_input(&input)?;
    println!("Valid.");
    Ok(())
}

/// Run `buzz cml canonicalize <path|-> [--output <path>]` locally.
pub fn cmd_canonicalize(path: &str, output: Option<&str>) -> Result<(), CliError> {
    let input = read_input(path)?;
    let canonical = canonicalize_input(&input)?;
    if let Some(output_path) = output {
        fs::write(output_path, canonical)
            .map_err(|error| CliError::Other(format!("failed to write {output_path}: {error}")))?;
    } else {
        print!("{canonical}");
    }
    Ok(())
}

fn read_input(path: &str) -> Result<String, CliError> {
    if path == "-" {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|error| CliError::Other(format!("failed to read stdin: {error}")))?;
        return Ok(input);
    }
    fs::read_to_string(path)
        .map_err(|error| CliError::Usage(format!("failed to read {path}: {error}")))
}

/// Map a CLI transition name to its typed transition and actor role.
fn parse_transition(name: &str) -> Result<(CmlTransition, CmlRole), CliError> {
    use buzz_core::cml_event::{CmlRole as R, CmlTransition as T};
    Ok(match name {
        "plan" => (T::Plan, R::Planner),
        "claim" => (T::Claim, R::Worker),
        "start" => (T::Start, R::Worker),
        "submit" => (T::Submit, R::Worker),
        "block" => (T::Block, R::Worker),
        "reject" => (T::ReviewReject, R::Reviewer),
        "fix-submit" => (T::FixSubmit, R::Fixer),
        "approve" => (T::ReviewApprove, R::Reviewer),
        "merge" => (T::Merge, R::Planner),
        "prove" => (T::RuntimeProve, R::Reviewer),
        "cancel" => (T::Cancel, R::Planner),
        other => {
            return Err(CliError::Usage(format!(
                "unknown transition {other:?}; expected plan|claim|start|submit|block|reject|fix-submit|approve|merge|prove|cancel"
            )))
        }
    })
}

/// Run `buzz cml events publish` — sign and submit one lifecycle event.
pub async fn cmd_events_publish(
    client: &crate::client::BuzzClient,
    private_key: &str,
    transition: &str,
    channel: &str,
    task_file: &str,
    prev: Option<&str>,
) -> Result<(), CliError> {
    let keys = nostr::Keys::parse(private_key)
        .map_err(|error| CliError::Auth(format!("invalid private key: {error}")))?;
    let channel_id = uuid::Uuid::parse_str(channel)
        .map_err(|error| CliError::Usage(format!("invalid channel UUID: {error}")))?;
    let (transition, role) = parse_transition(transition)?;
    let task_text = read_input(task_file)?;
    let task = buzz_core::cml::parse_cml(&task_text)
        .map_err(|error| CliError::Usage(format!("invalid CML snapshot: {error}")))?;
    let previous = match prev {
        Some(hex) => Some(
            EventId::from_hex(hex)
                .map_err(|error| CliError::Usage(format!("invalid --prev event id: {error}")))?,
        ),
        None => None,
    };
    let builder = buzz_sdk::build_cml_transition(channel_id, &task, transition, role, previous)
        .map_err(|error| CliError::Usage(format!("invalid transition: {error}")))?;
    let event = builder
        .sign_with_keys(&keys)
        .map_err(|e| CliError::Other(format!("failed to sign transition event: {e}")))?;
    buzz_core::cml_event::validate_cml_event(&event)
        .map_err(|error| CliError::Usage(format!("self-check rejected event: {error}")))?;
    let event_id = event.id.to_hex();
    let raw = client
        .submit_event(event)
        .await
        .map_err(|error| CliError::Other(format!("relay rejected event: {error}")))?;
    crate::commands::parse_write_response(&raw, "duplicate CML transition")?;
    println!(r#"{{"accepted":true,"event_id":"{event_id}"}}"#);
    Ok(())
}

/// Run `buzz cml events reduce` — fetch and reduce a task's events.
pub async fn cmd_events_reduce(
    client: &crate::client::BuzzClient,
    channel: &str,
    task: &str,
) -> Result<(), CliError> {
    let channel_id = uuid::Uuid::parse_str(channel)
        .map_err(|error| CliError::Usage(format!("invalid channel UUID: {error}")))?;
    let task_id = uuid::Uuid::parse_str(task)
        .map_err(|error| CliError::Usage(format!("invalid task UUID: {error}")))?;
    let events = fetch_cml_events(client, channel_id, task_id).await?;
    let reduced = buzz_core::cml_event::reduce_cml_events(&events)
        .map_err(|error| CliError::Usage(format!("reduction failed: {error}")))?;
    let snapshot = reduced
        .task
        .to_canonical_json()
        .map_err(|error| CliError::Other(format!("serialize snapshot: {error}")))?;
    let verdict = if reduced.conflicted {
        "conflicted"
    } else {
        "ok"
    };
    println!(
        r#"{{"verdict":"{verdict}","head":"{}","snapshot":{}}}"#,
        reduced.head.to_hex(),
        snapshot.trim_end()
    );
    Ok(())
}

async fn fetch_cml_events(
    client: &crate::client::BuzzClient,
    channel: uuid::Uuid,
    task: uuid::Uuid,
) -> Result<Vec<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [43001, 43002, 43003, 43004, 43005, 43006],
        "#h": [channel.to_string()],
        "#d": [task.to_string()],
        "limit": 500,
    });
    let raw = client
        .query(&filter)
        .await
        .map_err(|error| CliError::Other(format!("relay query failed: {error}")))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("relay response not JSON: {error}")))?;
    let events = value
        .as_array()
        .ok_or_else(|| CliError::Other("relay response is not an event array".into()))?;
    events
        .iter()
        .map(|item| {
            serde_json::from_value(item.clone())
                .map_err(|error| CliError::Other(format!("invalid event in response: {error}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_input, validate_input};

    const VALID: &str = r#"{
      "acceptance": [], "blockers": [], "evidence": [],
      "git": {"base_sha":"1111111111111111111111111111111111111111","branch":"feat/cml","head_sha":null,"repo":"block/buzz","worktree_alias":"buzz-cml"},
      "id":"cdd4722d-7481-4d01-9c0a-423b4454c179","lease":null,
      "objective":"One outcome","priority":"P1","protocol":"buzz-cml",
      "review":{"max_rounds":3,"round":0},
      "roles":{"fixer":null,"planner":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","reviewer":null,"worker":null},
      "runtime":{"host_id":null,"last_heartbeat_at":null,"presence":"offline","ttl_seconds":180},
      "status":"proposed","title":"CML","updated_at":1787673000,"version":1
    }"#;

    #[test]
    fn local_validate_accepts_valid_cml_and_rejects_unknown_fields() {
        validate_input(VALID).expect("valid CML");
        let invalid = VALID.replace("\"version\":1", "\"version\":1,\"surprise\":true");
        assert!(validate_input(&invalid).is_err());
    }

    #[test]
    fn canonicalize_is_a_byte_stable_fixed_point() {
        let first = canonicalize_input(VALID).expect("canonicalize");
        let second = canonicalize_input(&first).expect("canonicalize again");
        assert_eq!(first, second);
        assert!(first.ends_with('\n'));
    }
}
