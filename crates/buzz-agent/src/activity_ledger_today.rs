use nostr::{Event, JsonUtil};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::Digest;

use crate::types::{ToolDef, ToolResult, ToolResultContent};

pub const ACTIVITY_LEDGER_TODAY_TOOL: &str = "get_activity_ledger_today";
const ACTIVITY_LEDGER_TODAY_SCHEMA: &str = "buzz.activity-ledger.today/v1";
const ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE: &str = "buzz.activity-ledger.today.read/v1";
const ACTIVITY_LEDGER_TODAY_PATH_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_PATH";
const ACTIVITY_LEDGER_TODAY_CAPABILITY_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_CAPABILITY";
const ACTIVITY_LEDGER_TODAY_OWNER_PUBKEY_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_OWNER_PUBKEY";
const ACTIVITY_LEDGER_TODAY_RELAY_URL_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_RELAY_URL";
const ACTIVITY_LEDGER_MAX_LIFETIME_SECS: u64 = 24 * 60 * 60;
const ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES: u64 = 8 * 1024 * 1024;
const ACTIVITY_LEDGER_MAX_FUTURE_GENERATED_AT_SECS: u64 = 300;
const ACTIVITY_LEDGER_DEFAULT_LIMIT: usize = 25;
const ACTIVITY_LEDGER_TODAY_SIGNED_KIND: u16 = 24202;
const ACTIVITY_LEDGER_TODAY_SIGNED_TAG_MARKER: &str = "buzz-activity-ledger-today";

pub fn activity_ledger_today_enabled() -> bool {
    env_non_empty(ACTIVITY_LEDGER_TODAY_PATH_ENV).is_some()
        && env_non_empty(ACTIVITY_LEDGER_TODAY_CAPABILITY_ENV).is_some()
        && env_non_empty(ACTIVITY_LEDGER_TODAY_OWNER_PUBKEY_ENV).is_some()
        && env_non_empty(ACTIVITY_LEDGER_TODAY_RELAY_URL_ENV).is_some()
}

pub fn activity_ledger_today_def() -> ToolDef {
    ToolDef {
        name: ACTIVITY_LEDGER_TODAY_TOOL.to_owned(),
        description: "Read the owner-authorized Buzz Activity Ledger Today snapshot from a local Desktop-produced file. Fails closed if the snapshot is missing, stale, misconfigured, or does not match the configured capability."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "channelId": {
                    "type": "string",
                    "description": "Optional exact channel id filter."
                },
                "agentPubkey": {
                    "type": "string",
                    "description": "Optional exact agent pubkey filter."
                },
                "status": {
                    "type": "string",
                    "description": "Optional exact mission journal status filter."
                },
                "proofState": {
                    "type": "string",
                    "description": "Optional exact proof state filter."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum journals to return, from 1 to 100."
                },
                "before": {
                    "type": "object",
                    "description": "Optional continuation cursor from nextBefore for the next older page.",
                    "properties": {
                        "endedAt": { "type": "string" },
                        "agentPubkey": { "type": "string" },
                        "id": { "type": "string" }
                    },
                    "required": ["endedAt", "agentPubkey", "id"],
                    "additionalProperties": false
                },
                "includeEvents": {
                    "type": "boolean",
                    "description": "When true, include each journal's normalized events. Defaults to false."
                }
            }
        }),
    }
}

pub async fn call_activity_ledger_today(arguments: &Value, max_text_bytes: usize) -> ToolResult {
    let Some(path) = env_non_empty(ACTIVITY_LEDGER_TODAY_PATH_ENV) else {
        return error_result(&format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: missing {ACTIVITY_LEDGER_TODAY_PATH_ENV}"
        ));
    };
    let Some(capability) = env_non_empty(ACTIVITY_LEDGER_TODAY_CAPABILITY_ENV) else {
        return error_result(&format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: missing {ACTIVITY_LEDGER_TODAY_CAPABILITY_ENV}"
        ));
    };
    let Some(expected_owner_pubkey) = env_non_empty(ACTIVITY_LEDGER_TODAY_OWNER_PUBKEY_ENV) else {
        return error_result(&format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: missing {ACTIVITY_LEDGER_TODAY_OWNER_PUBKEY_ENV}"
        ));
    };
    let Some(expected_relay_url) = env_non_empty(ACTIVITY_LEDGER_TODAY_RELAY_URL_ENV) else {
        return error_result(&format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: missing {ACTIVITY_LEDGER_TODAY_RELAY_URL_ENV}"
        ));
    };
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let arguments = arguments.clone();
    match tokio::task::spawn_blocking(move || {
        read_activity_ledger_today(
            &path,
            &capability,
            &expected_owner_pubkey,
            &expected_relay_url,
            &arguments,
            now_secs,
        )
    })
    .await
    .unwrap_or_else(|e| Err(format!("{ACTIVITY_LEDGER_TODAY_TOOL}: task failed: {e}")))
    {
        Ok(output) => success_result(output, max_text_bytes),
        Err(msg) => error_result(&msg),
    }
}

fn success_result(output: String, max_text_bytes: usize) -> ToolResult {
    if output.len() > max_text_bytes {
        return error_result(&format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: query result is {} bytes and exceeds the {max_text_bytes}-byte model text budget; retry with includeEvents false or a smaller limit, then use nextBefore to retrieve older pages",
            output.len()
        ));
    }
    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(output)],
        is_error: false,
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

#[derive(Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
struct ActivityLedgerCursor {
    ended_at: String,
    agent_pubkey: String,
    id: String,
}

#[derive(Clone)]
struct ActivityLedgerQuery {
    channel_id: Option<String>,
    agent_pubkey: Option<String>,
    status: Option<String>,
    proof_state: Option<String>,
    limit: usize,
    before: Option<ActivityLedgerCursor>,
    include_events: bool,
}

fn read_activity_ledger_today(
    path: &str,
    capability: &str,
    expected_owner_pubkey: &str,
    expected_relay_url: &str,
    arguments: &Value,
    now_secs: u64,
) -> Result<String, String> {
    let query = parse_activity_ledger_query(arguments)?;
    let path_buf = std::path::PathBuf::from(path);
    if !path_buf.is_absolute() {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot path must be absolute"
        ));
    }
    let body = read_activity_ledger_snapshot_body(&path_buf)?;
    let root: Value = serde_json::from_str(&body)
        .map_err(|e| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: invalid snapshot JSON: {e}"))?;
    let result = filter_activity_ledger_snapshot(
        &root,
        capability,
        expected_owner_pubkey,
        expected_relay_url,
        &query,
        now_secs,
    )?;
    serde_json::to_string_pretty(&result)
        .map_err(|e| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: could not serialize query result: {e}"))
}

#[cfg(unix)]
fn read_activity_ledger_snapshot_body(path: &std::path::Path) -> Result<String, String> {
    use nix::errno::Errno;
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::{fstat, Mode, SFlag};
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::PermissionsExt;

    let fd: OwnedFd =
        open(path, OFlag::O_RDONLY | OFlag::O_NOFOLLOW, Mode::empty()).map_err(|e| {
            if e == Errno::ELOOP {
                format!("{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot path must not be a symlink")
            } else {
                format!(
                    "{ACTIVITY_LEDGER_TODAY_TOOL}: could not open snapshot {:?}: {e}",
                    path
                )
            }
        })?;
    let stat = fstat(&fd).map_err(|e| {
        format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: could not stat opened snapshot {:?}: {e}",
            path
        )
    })?;
    if SFlag::from_bits_truncate(stat.st_mode) != SFlag::S_IFREG {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot path must be a regular file"
        ));
    }
    let mode = stat.st_mode & 0o777;
    if mode != 0o600 {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot mode must be 0600, got {:03o}",
            mode
        ));
    }

    let mut file = std::fs::File::from(fd);
    let metadata = file.metadata().map_err(|e| {
        format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: could not read snapshot metadata {:?}: {e}",
            path
        )
    })?;
    let file_mode = metadata.permissions().mode() & 0o777;
    if file_mode != 0o600 {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot mode must be 0600, got {:03o}",
            file_mode
        ));
    }
    if metadata.len() > ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot exceeds {ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES} bytes"
        ));
    }

    let mut body = String::new();
    file.read_to_string(&mut body).map_err(|e| {
        format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: could not read snapshot {:?}: {e}",
            path
        )
    })?;
    Ok(body)
}

#[cfg(not(unix))]
fn read_activity_ledger_snapshot_body(path: &std::path::Path) -> Result<String, String> {
    let symlink_meta = std::fs::symlink_metadata(path).map_err(|e| {
        format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: could not stat snapshot {:?}: {e}",
            path
        )
    })?;
    if symlink_meta.file_type().is_symlink() {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot path must not be a symlink"
        ));
    }
    if !symlink_meta.file_type().is_file() {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot path must be a regular file"
        ));
    }
    if symlink_meta.len() > ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot exceeds {ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES} bytes"
        ));
    }
    std::fs::read_to_string(path).map_err(|e| {
        format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: could not read snapshot {:?}: {e}",
            path
        )
    })
}

fn parse_activity_ledger_query(arguments: &Value) -> Result<ActivityLedgerQuery, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: arguments must be an object"))?;
    Ok(ActivityLedgerQuery {
        channel_id: optional_string_arg(object, "channelId")?,
        agent_pubkey: optional_string_arg(object, "agentPubkey")?,
        status: optional_string_arg(object, "status")?,
        proof_state: optional_string_arg(object, "proofState")?,
        limit: parse_limit_arg(object.get("limit"))?,
        before: parse_cursor_arg(object.get("before"))?,
        include_events: parse_bool_arg(object.get("includeEvents"))?,
    })
}

fn parse_cursor_arg(value: Option<&Value>) -> Result<Option<ActivityLedgerCursor>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: before must be a cursor object"))?;
    if object.len() != 3 {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: before must contain only endedAt, agentPubkey, and id"
        ));
    }
    Ok(Some(ActivityLedgerCursor {
        ended_at: required_string_field(object, "endedAt")?.to_owned(),
        agent_pubkey: required_string_field(object, "agentPubkey")?.to_owned(),
        id: required_string_field(object, "id")?.to_owned(),
    }))
}

fn optional_string_arg(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Err(format!(
                    "{ACTIVITY_LEDGER_TODAY_TOOL}: {key} must not be blank"
                ))
            } else {
                Ok(Some(trimmed.to_owned()))
            }
        }
        Some(_) => Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: {key} must be a string"
        )),
    }
}

fn parse_limit_arg(value: Option<&Value>) -> Result<usize, String> {
    match value {
        None | Some(Value::Null) => Ok(ACTIVITY_LEDGER_DEFAULT_LIMIT),
        Some(Value::Number(number)) => {
            let Some(limit) = number.as_u64() else {
                return Err(format!(
                    "{ACTIVITY_LEDGER_TODAY_TOOL}: limit must be an integer from 1 to 100"
                ));
            };
            if !(1..=100).contains(&limit) {
                return Err(format!(
                    "{ACTIVITY_LEDGER_TODAY_TOOL}: limit must be an integer from 1 to 100"
                ));
            }
            Ok(limit as usize)
        }
        Some(_) => Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: limit must be an integer from 1 to 100"
        )),
    }
}

fn parse_bool_arg(value: Option<&Value>) -> Result<bool, String> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: includeEvents must be a boolean"
        )),
    }
}

fn filter_activity_ledger_snapshot(
    root: &Value,
    capability: &str,
    expected_owner_pubkey: &str,
    expected_relay_url: &str,
    query: &ActivityLedgerQuery,
    now_secs: u64,
) -> Result<Value, String> {
    let object = root
        .as_object()
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot root must be an object"))?;
    require_string_field(object, "schema", ACTIVITY_LEDGER_TODAY_SCHEMA)?;
    require_string_field(object, "capability", capability)?;
    let owner_pubkey = required_string_field(object, "ownerPubkey")?;
    if !is_hex_64(owner_pubkey) {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: ownerPubkey must be 64 lowercase hex chars"
        ));
    }
    if owner_pubkey != expected_owner_pubkey {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: ownerPubkey mismatch: expected {expected_owner_pubkey:?}, got {owner_pubkey:?}"
        ));
    }
    let relay_url = required_string_field(object, "relayUrl")?;
    if relay_url != expected_relay_url {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: relayUrl mismatch: expected {expected_relay_url:?}, got {relay_url:?}"
        ));
    }
    let generated_at = required_u64_field(object, "generatedAt")?;
    let expires_at = required_u64_field(object, "expiresAt")?;
    if generated_at > now_secs.saturating_add(ACTIVITY_LEDGER_MAX_FUTURE_GENERATED_AT_SECS) {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: generatedAt is more than {} seconds in the future",
            ACTIVITY_LEDGER_MAX_FUTURE_GENERATED_AT_SECS
        ));
    }
    if expires_at <= generated_at {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: expiresAt must be greater than generatedAt"
        ));
    }
    if expires_at - generated_at > ACTIVITY_LEDGER_MAX_LIFETIME_SECS {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot lifetime exceeds {} seconds",
            ACTIVITY_LEDGER_MAX_LIFETIME_SECS
        ));
    }
    if now_secs >= expires_at {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot expired at {expires_at}"
        ));
    }
    let snapshot_sha256 = required_string_field(object, "snapshotSha256")?;
    require_lower_hex_len(snapshot_sha256, "snapshotSha256", 64)?;
    let event_id = required_string_field(object, "eventId")?;
    require_lower_hex_len(event_id, "eventId", 64)?;
    let signature = required_string_field(object, "signature")?;
    require_lower_hex_len(signature, "signature", 128)?;
    verify_activity_ledger_snapshot_signature(
        object,
        &ActivityLedgerSnapshotSignatureFields {
            owner_pubkey,
            relay_url,
            generated_at,
            expires_at,
            snapshot_sha256,
            event_id,
            signature,
        },
    )?;

    let surface = object
        .get("surface")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!("{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot surface must be an object")
        })?;
    let day = required_string_field(surface, "day")?.to_owned();
    let journals = surface
        .get("journals")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!("{ACTIVITY_LEDGER_TODAY_TOOL}: surface.journals must be an array")
        })?;
    let source_projection = surface
        .get("snapshotProjection")
        .cloned()
        .unwrap_or(Value::Null);

    let mut filtered = Vec::new();
    for journal in journals {
        if journal_matches_query(journal, query)? {
            let projected = strip_events_from_journal(journal, query.include_events)?;
            journal_cursor(&projected)?;
            filtered.push(projected);
        }
    }
    let matching_journals = filtered.len();
    filtered.sort_by(|left, right| journal_sort_key(left).cmp(&journal_sort_key(right)));
    if let Some(before) = &query.before {
        filtered.retain(|journal| journal_cursor(journal).is_ok_and(|cursor| &cursor < before));
    }
    let eligible_before_cursor = filtered.len();
    let truncated = eligible_before_cursor > query.limit;
    if truncated {
        let oldest_to_drop = filtered.len() - query.limit;
        filtered.drain(..oldest_to_drop);
    }
    let next_before = if truncated {
        filtered.first().map(journal_cursor).transpose()?
    } else {
        None
    };

    let channels = rebuild_filtered_channels(&filtered);
    let failed = filtered
        .iter()
        .filter(|journal| string_field(journal, "status") == Some("failed"))
        .count();
    let in_progress = filtered
        .iter()
        .filter(|journal| string_field(journal, "status") == Some("in_progress"))
        .count();
    let claimed_without_evidence = filtered
        .iter()
        .filter(|journal| bool_field(journal, "claimedCompletionWithoutEvidence"))
        .count();

    Ok(json!({
        "schema": "buzz.activity-ledger.today.query-result/v1",
        "sourceSchema": ACTIVITY_LEDGER_TODAY_SCHEMA,
        "day": day,
        "ownerPubkey": owner_pubkey,
        "relayUrl": relay_url,
        "generatedAt": generated_at,
        "expiresAt": expires_at,
        "capability": capability,
        "sourceProjection": source_projection,
        "filters": {
            "channelId": query.channel_id,
            "agentPubkey": query.agent_pubkey,
            "status": query.status,
            "proofState": query.proof_state,
            "limit": query.limit,
            "before": query.before.clone(),
            "includeEvents": query.include_events,
        },
        "counts": {
            "matchingJournals": matching_journals,
            "eligibleBeforeCursor": eligible_before_cursor,
            "returnedJournals": filtered.len(),
            "failed": failed,
            "inProgress": in_progress,
            "claimedWithoutEvidence": claimed_without_evidence,
        },
        "truncated": truncated,
        "nextBefore": next_before,
        "journals": filtered,
        "channels": channels,
    }))
}

fn journal_cursor(journal: &Value) -> Result<ActivityLedgerCursor, String> {
    let object = journal
        .as_object()
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: each journal must be an object"))?;
    Ok(ActivityLedgerCursor {
        ended_at: required_string_field(object, "endedAt")?.to_owned(),
        agent_pubkey: required_string_field(object, "agentPubkey")?.to_owned(),
        id: required_string_field(object, "id")?.to_owned(),
    })
}

fn journal_sort_key(journal: &Value) -> (&str, &str, &str) {
    (
        string_field(journal, "endedAt").unwrap_or_default(),
        string_field(journal, "agentPubkey").unwrap_or_default(),
        string_field(journal, "id").unwrap_or_default(),
    )
}

fn journal_matches_query(journal: &Value, query: &ActivityLedgerQuery) -> Result<bool, String> {
    let object = journal
        .as_object()
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: each journal must be an object"))?;
    if let Some(channel_id) = &query.channel_id {
        if object.get("channelId").and_then(Value::as_str) != Some(channel_id.as_str()) {
            return Ok(false);
        }
    }
    if let Some(agent_pubkey) = &query.agent_pubkey {
        if object.get("agentPubkey").and_then(Value::as_str) != Some(agent_pubkey.as_str()) {
            return Ok(false);
        }
    }
    if let Some(status) = &query.status {
        if object.get("status").and_then(Value::as_str) != Some(status.as_str()) {
            return Ok(false);
        }
    }
    if let Some(proof_state) = &query.proof_state {
        if object.get("proofState").and_then(Value::as_str) != Some(proof_state.as_str()) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn strip_events_from_journal(journal: &Value, include_events: bool) -> Result<Value, String> {
    if include_events {
        return Ok(journal.clone());
    }
    let mut object = journal
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: each journal must be an object"))?;
    object.remove("events");
    Ok(Value::Object(object))
}

type FilteredChannelSummary = (
    Vec<String>,
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
    String,
);

fn rebuild_filtered_channels(journals: &[Value]) -> Vec<Value> {
    let mut channels: std::collections::BTreeMap<String, FilteredChannelSummary> =
        std::collections::BTreeMap::new();

    for journal in journals {
        let Some(channel_id) = string_field(journal, "channelId").map(str::to_owned) else {
            continue;
        };
        let journal_id = string_field(journal, "id").unwrap_or_default().to_owned();
        let agent_pubkey = string_field(journal, "agentPubkey")
            .unwrap_or_default()
            .to_owned();
        let agent_name = string_field(journal, "agentName")
            .unwrap_or_default()
            .to_owned();
        let ended_at = string_field(journal, "endedAt")
            .unwrap_or_default()
            .to_owned();

        let entry = channels.entry(channel_id).or_insert_with(|| {
            (
                Vec::new(),
                std::collections::BTreeSet::new(),
                std::collections::BTreeSet::new(),
                ended_at.clone(),
            )
        });
        entry.0.push(journal_id);
        if !agent_pubkey.is_empty() {
            entry.1.insert(agent_pubkey);
        }
        if !agent_name.is_empty() {
            entry.2.insert(agent_name);
        }
        if ended_at > entry.3 {
            entry.3 = ended_at;
        }
    }

    channels
        .into_iter()
        .map(
            |(channel_id, (journal_ids, agent_pubkeys, agent_names, last_activity_at))| {
                json!({
                    "channelId": channel_id,
                    "journalIds": journal_ids,
                    "agentPubkeys": agent_pubkeys.into_iter().collect::<Vec<_>>(),
                    "agentNames": agent_names.into_iter().collect::<Vec<_>>(),
                    "lastActivityAt": last_activity_at,
                })
            },
        )
        .collect()
}

fn require_string_field(
    object: &Map<String, Value>,
    key: &str,
    expected: &str,
) -> Result<(), String> {
    let value = required_string_field(object, key)?;
    if value != expected {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: {key} mismatch: expected {expected:?}, got {value:?}"
        ));
    }
    Ok(())
}

fn require_lower_hex_len(value: &str, key: &str, len: usize) -> Result<(), String> {
    if value.len() != len
        || !value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: {key} must be {len} lowercase hex chars"
        ));
    }
    Ok(())
}

fn required_string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: missing string field {key}"))
}

fn required_u64_field(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: missing integer field {key}"))
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.as_object()?.get(key)?.as_str()
}

fn bool_field(value: &Value, key: &str) -> bool {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn is_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

struct ActivityLedgerSnapshotSignatureFields<'a> {
    owner_pubkey: &'a str,
    relay_url: &'a str,
    generated_at: u64,
    expires_at: u64,
    snapshot_sha256: &'a str,
    event_id: &'a str,
    signature: &'a str,
}

fn verify_activity_ledger_snapshot_signature(
    object: &Map<String, Value>,
    fields: &ActivityLedgerSnapshotSignatureFields<'_>,
) -> Result<(), String> {
    let payload_json = canonical_activity_ledger_snapshot_payload_json(
        object,
        fields.owner_pubkey,
        fields.relay_url,
        fields.generated_at,
        fields.expires_at,
    )?;
    let payload_sha256 = hex::encode(sha2::Sha256::digest(payload_json.as_bytes()));
    if payload_sha256 != fields.snapshot_sha256 {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshotSha256 does not match the canonical payload"
        ));
    }
    let event_json = serde_json::json!({
        "id": fields.event_id,
        "pubkey": fields.owner_pubkey,
        "created_at": fields.generated_at,
        "kind": ACTIVITY_LEDGER_TODAY_SIGNED_KIND,
        "tags": [
            ["t", ACTIVITY_LEDGER_TODAY_SIGNED_TAG_MARKER],
            ["schema", ACTIVITY_LEDGER_TODAY_SCHEMA],
            ["capability", ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE],
            ["snapshot_sha256", fields.snapshot_sha256],
            ["expires_at", fields.expires_at.to_string()]
        ],
        "content": payload_json,
        "sig": fields.signature,
    })
    .to_string();
    let event = Event::from_json(&event_json)
        .map_err(|e| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: invalid signed snapshot event: {e}"))?;
    event.verify().map_err(|e| {
        format!("{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot signature verification failed: {e}")
    })?;
    if event.id.to_hex() != fields.event_id {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: eventId does not match the signed snapshot event"
        ));
    }
    if event.pubkey.to_hex() != fields.owner_pubkey {
        return Err(format!(
            "{ACTIVITY_LEDGER_TODAY_TOOL}: snapshot signer is not the expected owner"
        ));
    }
    Ok(())
}

fn canonical_activity_ledger_snapshot_payload_json(
    object: &Map<String, Value>,
    owner_pubkey: &str,
    relay_url: &str,
    generated_at: u64,
    expires_at: u64,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalPayload<'a> {
        schema: &'static str,
        owner_pubkey: &'a str,
        relay_url: &'a str,
        generated_at: u64,
        expires_at: u64,
        capability: &'static str,
        surface: &'a Value,
        raw_events: &'a [Value],
    }

    let surface = object
        .get("surface")
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: missing surface field"))?;
    let raw_events = object
        .get("rawEvents")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{ACTIVITY_LEDGER_TODAY_TOOL}: missing rawEvents field"))?;
    serde_json::to_string(&CanonicalPayload {
        schema: ACTIVITY_LEDGER_TODAY_SCHEMA,
        owner_pubkey,
        relay_url,
        generated_at,
        expires_at,
        capability: ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE,
        surface,
        raw_events,
    })
    .map_err(|e| {
        format!("{ACTIVITY_LEDGER_TODAY_TOOL}: could not canonicalize snapshot payload: {e}")
    })
}

fn error_result(msg: &str) -> ToolResult {
    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(msg.to_owned())],
        is_error: true,
    }
}

#[cfg(test)]
#[path = "activity_ledger_today_tests.rs"]
mod tests;
