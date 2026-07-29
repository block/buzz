use std::io::Write;
use std::process::{Command, Stdio};

use nostr::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::artifact_sync;
use super::profile::{ProfileEnvironment, ResolvedProfile};
use super::runtime::{
    event_builder, hostname, now_secs, parse_id, query_events, read_text, tag, ContextRuntime,
};
use crate::error::CliError;

const LIFECYCLE_TAGS: [&str; 4] = [
    "handoff:open",
    "handoff:claim",
    "handoff:return",
    "handoff:close",
];

#[derive(Debug, Deserialize, Serialize)]
struct HandoffState {
    open_id: String,
    title: String,
    context: String,
    opener_owner_pubkey: String,
    allowed_claimants: Vec<String>,
    created_at: u64,
    updated_at: u64,
    state: String,
    claim_id: Option<String>,
    claimant_pubkey: Option<String>,
    claim_created_at: Option<u64>,
    return_id: Option<String>,
    return_created_at: Option<u64>,
    #[serde(default)]
    conflicting_claims: Vec<String>,
    #[serde(default)]
    ignored: Vec<Value>,
}

pub async fn open(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    spec: &std::path::Path,
) -> Result<String, CliError> {
    let content = read_text(Some(spec))?;
    let value = parse_object(&content, "handoff open")?;
    for field in ["title", "scope", "base_commit", "acceptance", "embodiment"] {
        require_field(&value, field, "handoff open")?;
    }
    let base_commit = require_string(&value, "base_commit", "handoff open")?;
    validate_lower_hex(base_commit, 40, "handoff open base_commit")?;
    let target = value
        .get("target")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::Usage("handoff open target must be an object".into()))?;
    let target_pubkey = require_string(target, "pubkey", "handoff open target")?;
    validate_lower_hex(target_pubkey, 64, "handoff open target.pubkey")?;
    let embodiment = value
        .get("embodiment")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::Usage("handoff open embodiment must be an object".into()))?;
    for field in ["stdin", "network", "trust", "tooling"] {
        require_field(embodiment, field, "handoff open embodiment")?;
    }
    let artifacts = artifacts(&value, "handoff open")?;

    let runtime = ContextRuntime::new(profile, environment)?;
    let context = runtime.default_context()?;
    let (_, role) = runtime.local_event_client()?;
    let identity = runtime.identity_label(role);
    let machine = hostname();
    let mut tags = vec![
        tag(&["t", "handoff:open"])?,
        tag(&["h", context])?,
        tag(&["p", target_pubkey])?,
        tag(&["agent", &identity])?,
        tag(&["machine", &machine])?,
    ];
    tags.extend(
        artifacts
            .iter()
            .map(|hash| tag(&["x", hash]))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let event_id = runtime
        .post_builder(event_builder(content, tags, None))
        .await?;
    println!("handoff opened: {event_id}");
    println!("context: {context}");
    Ok(event_id)
}

pub async fn claim(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    open_id: &str,
    note: Option<&str>,
) -> Result<String, CliError> {
    let open_id = parse_id(open_id, "open event id")?;
    let runtime = ContextRuntime::new(profile, environment)?;
    let (client, role) = runtime.local_event_client()?;
    let signer = client.keys().public_key().to_hex();
    let state = state_for(profile, environment, &open_id).await?;
    match state.state.as_str() {
        "OPEN" => {
            if !state.allowed_claimants.contains(&signer) {
                return Err(CliError::Auth(format!(
                    "signer {signer} is not authorized to claim this handoff"
                )));
            }
        }
        "CLAIMED" if state.claimant_pubkey.as_deref() == Some(signer.as_str()) => {
            let claim_id = state.claim_id.ok_or_else(|| {
                CliError::Other("reducer reported CLAIMED without a claim id".into())
            })?;
            println!("handoff already claimed: {open_id}");
            println!("claim event: {claim_id}");
            return Ok(claim_id);
        }
        "CLAIMED" => {
            return Err(CliError::Conflict(format!(
                "handoff is already claimed by {}",
                state.claimant_pubkey.as_deref().unwrap_or("unknown")
            )));
        }
        "CONFLICT" => {
            return Err(CliError::Conflict(
                "handoff has conflicting authorized claims; manual resolution is required".into(),
            ));
        }
        state => {
            return Err(CliError::Conflict(format!(
                "handoff cannot be claimed from lifecycle state {state}"
            )));
        }
    }

    let identity = runtime.identity_label(role);
    let machine = hostname();
    let mut content = serde_json::json!({
        "runner": identity,
        "host": machine,
    });
    if let Some(note) = note.filter(|note| !note.is_empty()) {
        content["note"] = Value::String(note.into());
    }
    let event_id = runtime
        .post_builder(event_builder(
            serde_json::to_string(&content).map_err(json_error)?,
            [
                tag(&["t", "handoff:claim"])?,
                tag(&["h", &state.context])?,
                tag(&["agent", &identity])?,
                tag(&["machine", &machine])?,
                tag(&["e", &open_id, "", "root"])?,
            ],
            Some(causal_timestamp(state.created_at)),
        ))
        .await?;
    println!("handoff claimed: {open_id} by {identity}@{machine}");
    println!("claim event: {event_id}");
    Ok(event_id)
}

pub async fn return_handoff(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    open_id: &str,
    spec: &std::path::Path,
) -> Result<String, CliError> {
    let open_id = parse_id(open_id, "open event id")?;
    let raw = read_text(Some(spec))?;
    let mut content = parse_object(&raw, "handoff return")?;
    let status = require_string(&content, "status", "handoff return")?.to_string();
    if !matches!(status.as_str(), "done" | "failed") {
        return Err(CliError::Usage(
            "handoff return status must be done or failed".into(),
        ));
    }
    require_field(&content, "evidence", "handoff return")?;
    let artifact_hashes = artifacts(&content, "handoff return")?;

    let runtime = ContextRuntime::new(profile, environment)?;
    let (client, role) = runtime.local_event_client()?;
    let signer = client.keys().public_key().to_hex();
    let state = state_for(profile, environment, &open_id).await?;
    if state.state != "CLAIMED" {
        return Err(CliError::Conflict(format!(
            "handoff return requires CLAIMED state, found {}",
            state.state
        )));
    }
    if state.claimant_pubkey.as_deref() != Some(signer.as_str()) {
        return Err(CliError::Auth(format!(
            "return signer {signer} is not claimant {}",
            state.claimant_pubkey.as_deref().unwrap_or("unknown")
        )));
    }
    let claim_id = state
        .claim_id
        .ok_or_else(|| CliError::Other("CLAIMED handoff has no claim id".into()))?;
    let claim_created_at = state
        .claim_created_at
        .ok_or_else(|| CliError::Other("CLAIMED handoff has no claim timestamp".into()))?;
    content.insert("claim_id".into(), Value::String(claim_id.clone()));
    for hash in &artifact_hashes {
        artifact_sync::verify_rendezvous_manifest(profile, environment, hash, &state.context)
            .await?;
    }

    let identity = runtime.identity_label(role);
    let machine = hostname();
    let mut tags = vec![
        tag(&["t", "handoff:return"])?,
        tag(&["h", &state.context])?,
        tag(&["agent", &identity])?,
        tag(&["machine", &machine])?,
        tag(&["e", &open_id, "", "root"])?,
        tag(&["e", &claim_id, "", "claim"])?,
    ];
    tags.extend(
        artifact_hashes
            .iter()
            .map(|hash| tag(&["x", hash]))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let event_id = runtime
        .post_builder(event_builder(
            serde_json::to_string(&content).map_err(json_error)?,
            tags,
            Some(causal_timestamp(claim_created_at)),
        ))
        .await?;
    println!("handoff returned: {open_id} ({status})");
    println!("return event: {event_id}");
    Ok(event_id)
}

pub async fn close(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    open_id: &str,
    return_id: &str,
    note: Option<&str>,
) -> Result<String, CliError> {
    let open_id = parse_id(open_id, "open event id")?;
    let return_id = parse_id(return_id, "return event id")?;
    let runtime = ContextRuntime::new(profile, environment)?;
    let (client, role) = runtime.local_event_client()?;
    let signer_owner = client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.keys().public_key().to_hex());
    let state = state_for(profile, environment, &open_id).await?;
    if state.state != "RETURNED" {
        return Err(CliError::Conflict(format!(
            "handoff close requires RETURNED state, found {}",
            state.state
        )));
    }
    if state.return_id.as_deref() != Some(return_id.as_str()) {
        return Err(CliError::Conflict(format!(
            "close targets {return_id} but current return is {}",
            state.return_id.as_deref().unwrap_or("absent")
        )));
    }
    if signer_owner != state.opener_owner_pubkey {
        return Err(CliError::Auth(format!(
            "close signer owner {signer_owner} is not opener owner {}",
            state.opener_owner_pubkey
        )));
    }
    verify_artifacts(profile, environment, &return_id).await?;
    let returned_at = state
        .return_created_at
        .ok_or_else(|| CliError::Other("RETURNED handoff has no return timestamp".into()))?;
    let identity = runtime.identity_label(role);
    let machine = hostname();
    let mut content = serde_json::json!({
        "verifier": identity,
        "host": machine,
        "return_id": return_id,
    });
    if let Some(note) = note.filter(|note| !note.is_empty()) {
        content["note"] = Value::String(note.into());
    }
    let event_id = runtime
        .post_builder(event_builder(
            serde_json::to_string(&content).map_err(json_error)?,
            [
                tag(&["t", "handoff:close"])?,
                tag(&["h", &state.context])?,
                tag(&["agent", &identity])?,
                tag(&["machine", &machine])?,
                tag(&["e", &open_id, "", "root"])?,
                tag(&["e", &return_id, "", "return"])?,
            ],
            Some(causal_timestamp(returned_at)),
        ))
        .await?;
    println!("handoff closed: {open_id} by {identity}@{machine}");
    println!("close event: {event_id}");
    Ok(event_id)
}

pub async fn verify_artifacts(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    return_id: &str,
) -> Result<(), CliError> {
    let return_id = parse_id(return_id, "return event id")?;
    let runtime = ContextRuntime::new(profile, environment)?;
    let reader = runtime.cloud_reader_client()?;
    let events = query_events(&reader, &lifecycle_filters()).await?;
    let states = reduce(profile, &events, None)?;
    let state = states
        .into_iter()
        .find(|state| {
            matches!(state.state.as_str(), "RETURNED" | "CLOSED")
                && state.return_id.as_deref() == Some(return_id.as_str())
        })
        .ok_or_else(|| {
            CliError::Conflict(format!(
                "return {return_id} is not cryptographically and causally valid in the rendezvous view"
            ))
        })?;
    let returned = events
        .iter()
        .filter(|event| {
            event.id.to_hex() == return_id
                && tag_values(event, "t").as_slice() == ["handoff:return"]
        })
        .collect::<Vec<_>>();
    if returned.len() != 1 {
        return Err(CliError::Conflict(format!(
            "rendezvous did not return exactly one handoff:return {return_id}"
        )));
    }
    let content = parse_object(&returned[0].content, "handoff return")?;
    let mut content_hashes = artifacts(&content, "handoff return")?;
    let mut tagged_hashes = tag_values(returned[0], "x");
    content_hashes.sort();
    tagged_hashes.sort();
    if content_hashes != tagged_hashes {
        return Err(CliError::Conflict(
            "return content artifacts do not match its x tags".into(),
        ));
    }
    for hash in &content_hashes {
        artifact_sync::verify_rendezvous_manifest(profile, environment, hash, &state.context)
            .await?;
        println!("verified artifact: {hash}");
    }
    println!("return custody verified: {return_id}");
    Ok(())
}

pub async fn list(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    json: bool,
) -> Result<(), CliError> {
    let events = lifecycle_events(profile, environment).await?;
    let states = reduce(profile, &events, None)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&states).map_err(json_error)?
        );
        return Ok(());
    }
    for state in states {
        let suffix = match state.state.as_str() {
            "CLOSED" => "  cryptographically verified".into(),
            "CONFLICT" => format!("  claims={}", state.conflicting_claims.join(",")),
            "INVALID" => format!(
                "  {}",
                state
                    .ignored
                    .first()
                    .and_then(|value| value.get("reason"))
                    .and_then(Value::as_str)
                    .unwrap_or("invalid open")
            ),
            _ => state
                .claimant_pubkey
                .as_deref()
                .map(|claimant| format!("  claimant={claimant}"))
                .unwrap_or_default(),
        };
        println!(
            "{}  {}  {}  {}{}",
            state.state,
            age(state.updated_at),
            state.title,
            state.open_id,
            suffix
        );
    }
    Ok(())
}

async fn state_for(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    open_id: &str,
) -> Result<HandoffState, CliError> {
    let events = lifecycle_events(profile, environment).await?;
    reduce(profile, &events, Some(open_id))?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NotFound(format!("handoff open event {open_id} was not found")))
}

async fn lifecycle_events(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
) -> Result<Vec<Event>, CliError> {
    ContextRuntime::new(profile, environment)?
        .query_union(&lifecycle_filters())
        .await
}

fn lifecycle_filters() -> Vec<Value> {
    vec![serde_json::json!({
        "kinds": [nostr::Kind::TextNote.as_u16()],
        "#t": LIFECYCLE_TAGS,
    })]
}

fn reduce(
    profile: &ResolvedProfile,
    events: &[Event],
    open_id: Option<&str>,
) -> Result<Vec<HandoffState>, CliError> {
    let executable = profile
        .file
        .runtime
        .handoff_reducer
        .as_deref()
        .ok_or_else(|| {
            CliError::Usage(format!(
                "profile {} does not configure runtime.handoff_reducer",
                profile.name
            ))
        })?;
    if !executable.is_file() {
        return Err(CliError::NotFound(format!(
            "configured handoff reducer is absent: {}",
            executable.display()
        )));
    }
    let mut command = Command::new(executable);
    if let Some(open_id) = open_id {
        command.arg("--open").arg(open_id);
    }
    let mut child = command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            CliError::Other(format!(
                "could not start handoff reducer {}: {error}",
                executable.display()
            ))
        })?;
    let input = serde_json::to_vec(events).map_err(json_error)?;
    child
        .stdin
        .take()
        .ok_or_else(|| CliError::Other("handoff reducer stdin was unavailable".into()))?
        .write_all(&input)
        .map_err(|error| CliError::Other(format!("could not feed handoff reducer: {error}")))?;
    let output = child.wait_with_output().map_err(|error| {
        CliError::Other(format!(
            "could not wait for handoff reducer {}: {error}",
            executable.display()
        ))
    })?;
    if !output.status.success() {
        return Err(CliError::Conflict(format!(
            "handoff reducer rejected the lifecycle: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if open_id.is_some() {
        let state: HandoffState = serde_json::from_slice(&output.stdout).map_err(|error| {
            CliError::Other(format!("handoff reducer output is invalid: {error}"))
        })?;
        Ok(vec![state])
    } else {
        serde_json::from_slice(&output.stdout)
            .map_err(|error| CliError::Other(format!("handoff reducer output is invalid: {error}")))
    }
}

fn parse_object(raw: &str, label: &str) -> Result<Map<String, Value>, CliError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| CliError::Usage(format!("{label} spec is invalid JSON: {error}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| CliError::Usage(format!("{label} spec must be a JSON object")))
}

fn require_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a Value, CliError> {
    object
        .get(field)
        .filter(|value| !value.is_null())
        .ok_or_else(|| CliError::Usage(format!("{label} spec missing required field: {field}")))
}

fn require_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a str, CliError> {
    require_field(object, field, label)?
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::Usage(format!("{label} field {field} must be a nonempty string")))
}

fn artifacts(object: &Map<String, Value>, label: &str) -> Result<Vec<String>, CliError> {
    let values = match object.get("artifacts") {
        None => return Ok(Vec::new()),
        Some(Value::Array(values)) => values,
        Some(_) => {
            return Err(CliError::Usage(format!(
                "{label} artifacts must be an array"
            )));
        }
    };
    values
        .iter()
        .map(|value| {
            let value = value.as_str().ok_or_else(|| {
                CliError::Usage(format!("{label} artifact hashes must be strings"))
            })?;
            validate_lower_hex(value, 64, &format!("{label} artifact"))?;
            Ok(value.to_string())
        })
        .collect()
}

fn validate_lower_hex(value: &str, length: usize, label: &str) -> Result<(), CliError> {
    if value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(CliError::Usage(format!(
            "{label} must be {length} lowercase hexadecimal characters"
        )))
    }
}

fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some(name))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .collect()
}

fn causal_timestamp(predecessor: u64) -> u64 {
    now_secs().max(predecessor.saturating_add(1))
}

fn age(timestamp: u64) -> String {
    let seconds = now_secs().saturating_sub(timestamp);
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3_600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3_600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

fn json_error(error: serde_json::Error) -> CliError {
    CliError::Other(format!("JSON serialization failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_specs_reject_noncanonical_artifacts() {
        let object = serde_json::json!({"artifacts": ["A".repeat(64)]});
        assert!(artifacts(object.as_object().expect("object"), "return").is_err());
    }

    #[test]
    fn causal_time_is_strictly_after_predecessor() {
        assert_eq!(causal_timestamp(u64::MAX), u64::MAX);
        assert!(causal_timestamp(now_secs() + 10) > now_secs());
    }
}
