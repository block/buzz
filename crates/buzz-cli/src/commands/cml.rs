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

/// Project a reduced CML task into its [`buzz_core::cml_view::WorkstreamCard`]
/// serialized as one line of JSON, as observed at `observed_at`.
///
/// Liveness, lease, and short-SHA derivation are delegated to
/// `buzz_core::cml_view::project_workstream_card` so the core reducer stays
/// the single authority for those rules. The rendered card never contains
/// absolute paths or full commit SHAs.
pub fn render_workstream_card(
    task: &buzz_core::cml::CmlTask,
    observed_at: u64,
) -> Result<String, CliError> {
    let card = buzz_core::cml_view::project_workstream_card(task, observed_at);
    serde_json::to_string(&card)
        .map_err(|error| CliError::Other(format!("serialize card: {error}")))
}

/// Run `buzz cml events card` — print a task's observation-time card.
///
/// Fetches the task's lifecycle events, reduces them, and renders the
/// workstream card observed at `as_of`, defaulting to the current
/// wall-clock time when `as_of` is `None`.
pub async fn cmd_events_card(
    client: &crate::client::BuzzClient,
    channel: &str,
    task: &str,
    as_of: Option<u64>,
) -> Result<(), CliError> {
    let channel_id = uuid::Uuid::parse_str(channel)
        .map_err(|error| CliError::Usage(format!("invalid channel UUID: {error}")))?;
    let task_id = uuid::Uuid::parse_str(task)
        .map_err(|error| CliError::Usage(format!("invalid task UUID: {error}")))?;
    let events = fetch_cml_events(client, channel_id, task_id).await?;
    let reduced = buzz_core::cml_event::reduce_cml_events(&events)
        .map_err(|error| CliError::Usage(format!("reduction failed: {error}")))?;
    let observed_at = as_of.unwrap_or_else(now_secs);
    let card = render_workstream_card(&reduced.task, observed_at)?;
    println!("{card}");
    Ok(())
}

/// Current wall-clock unix seconds; 0 if the system clock predates the epoch.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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
    use super::{canonicalize_input, render_workstream_card, validate_input};

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

    /// A working task with a live worker: heartbeat at 1787673000, TTL 180,
    /// lease held by the assigned worker until 1787674000, head commit set.
    fn live_task_json() -> String {
        let planner = "a".repeat(64);
        let worker = "b".repeat(64);
        format!(
            r#"{{
      "acceptance": [], "blockers": [], "evidence": [],
      "git": {{"base_sha":"1111111111111111111111111111111111111111","branch":"feat/cml","head_sha":"2222222222222222222222222222222222222222","repo":"block/buzz","worktree_alias":"buzz-cml"}},
      "id":"cdd4722d-7481-4d01-9c0a-423b4454c179",
      "lease":{{"id":"lease-1","holder":"{worker}","issued_at":1787673000,"expires_at":1787674000}},
      "objective":"One outcome","priority":"P1","protocol":"buzz-cml",
      "review":{{"max_rounds":3,"round":0}},
      "roles":{{"fixer":null,"planner":"{planner}","reviewer":null,"worker":"{worker}"}},
      "runtime":{{"host_id":"h_0123456789abcdef","last_heartbeat_at":1787673000,"presence":"online","ttl_seconds":180}},
      "status":"working","title":"CML","updated_at":1787673000,"version":1
    }}"#
        )
    }

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

    #[test]
    fn card_projects_live_task_values_at_observation_time() {
        let task = buzz_core::cml::parse_cml(&live_task_json()).expect("valid live CML");
        let card = render_workstream_card(&task, 1787673060).expect("render card");
        assert!(card.contains(r#""title":"CML""#), "card: {card}");
        assert!(card.contains(r#""status":"working""#), "card: {card}");
        assert!(card.contains(r#""liveness":"online""#), "card: {card}");
        assert!(card.contains(r#""live_claim":true"#), "card: {card}");
        assert!(card.contains(r#""base_short":"1111111""#), "card: {card}");
        assert!(card.contains(r#""head_short":"2222222""#), "card: {card}");
        assert!(
            card.contains(r#""worktree_alias":"buzz-cml""#),
            "card: {card}"
        );
        assert!(!card.contains('\n'), "card must be a single line: {card}");
    }

    #[test]
    fn card_liveness_follows_observation_time_not_snapshot_presence() {
        let task = buzz_core::cml::parse_cml(&live_task_json()).expect("valid live CML");
        // Snapshot stores presence "online" at updated_at 1787673000; the card
        // must recompute against the observation instant instead of echoing it.
        let stale = render_workstream_card(&task, 1787673300).expect("render card");
        assert!(stale.contains(r#""liveness":"stale""#), "card: {stale}");
        assert!(stale.contains(r#""live_claim":false"#), "card: {stale}");
        let dead = render_workstream_card(&task, 1787674000).expect("render card");
        assert!(dead.contains(r#""liveness":"offline""#), "card: {dead}");
        assert!(dead.contains(r#""live_claim":false"#), "card: {dead}");
        assert!(dead.contains(r#""status":"working""#), "card: {dead}");
    }

    #[test]
    fn card_renders_null_head_when_no_head_commit_exists() {
        let task = buzz_core::cml::parse_cml(VALID).expect("valid CML");
        let card = render_workstream_card(&task, 1787674000).expect("render card");
        assert!(card.contains(r#""head_short":null"#), "card: {card}");
        assert!(card.contains(r#""liveness":"offline""#), "card: {card}");
        assert!(card.contains(r#""status":"proposed""#), "card: {card}");
        assert!(
            card.contains(r#""worktree_alias":"buzz-cml""#),
            "card: {card}"
        );
    }

    #[test]
    fn card_never_leaks_absolute_paths_or_full_shas() {
        let task = buzz_core::cml::parse_cml(&live_task_json()).expect("valid live CML");
        let card = render_workstream_card(&task, 1787673060).expect("render card");
        assert!(!card.contains("/private/tmp"), "card: {card}");
        assert!(!card.contains("/Users/"), "card: {card}");
        assert!(!card.contains("/tmp/"), "card: {card}");
        assert!(
            !card.contains("1111111111111111111111111111111111111111"),
            "full base SHA must not appear: {card}"
        );
        let value: serde_json::Value = serde_json::from_str(&card).expect("card is JSON");
        let base_short = value["base_short"]
            .as_str()
            .expect("base_short is a string");
        assert_eq!(base_short.len(), 7, "card: {card}");
        let head_short = value["head_short"]
            .as_str()
            .expect("head_short is a string");
        assert_eq!(head_short.len(), 7, "card: {card}");
    }
}
