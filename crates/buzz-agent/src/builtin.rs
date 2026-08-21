//! Built-in tools that run in-process, bypassing MCP.
//!
//! Currently: `load_skill` — reads a skill's full SKILL.md body from disk
//! and returns it so the agent can load skill content on demand rather than
//! having every skill inlined into the system prompt at session start.

use serde_json::{json, Map, Value};

use crate::hints::{strip_frontmatter, SkillEntry, MAX_SKILL_BODY_BYTES};
use crate::mcp::truncate_at_boundary;
use crate::types::{ToolDef, ToolResult, ToolResultContent};

pub const LOAD_SKILL_TOOL: &str = "load_skill";
pub const ACTIVITY_LEDGER_TODAY_TOOL: &str = "get_activity_ledger_today";
const ACTIVITY_LEDGER_TODAY_SCHEMA: &str = "buzz.activity-ledger.today/v1";
const ACTIVITY_LEDGER_TODAY_PATH_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_PATH";
const ACTIVITY_LEDGER_TODAY_CAPABILITY_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_CAPABILITY";
const ACTIVITY_LEDGER_MAX_LIFETIME_SECS: u64 = 24 * 60 * 60;
const ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES: u64 = 8 * 1024 * 1024;
const ACTIVITY_LEDGER_MAX_FUTURE_GENERATED_AT_SECS: u64 = 300;
const ACTIVITY_LEDGER_DEFAULT_LIMIT: usize = 25;

/// Return the `ToolDef` for `load_skill` to include in the LLM tool list.
pub fn load_skill_def() -> ToolDef {
    ToolDef {
        name: LOAD_SKILL_TOOL.to_owned(),
        description: "Load the full content of a skill by name. \
            Call this before using a skill — the system prompt lists skill names \
            and descriptions only; the full instructions are loaded on demand. \
            To load a supporting file within a skill, use the form \
            \"skill-name/relative/path\" (e.g. \"my-skill/references/foo.md\")."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill name as listed in the Available Skills section, \
                        or \"skill-name/relative/path\" to load a supporting file."
                }
            },
            "required": ["name"]
        }),
    }
}

/// Returns true when the runtime was explicitly provisioned with a local
/// Desktop-authored Today snapshot and matching capability marker.
pub fn activity_ledger_today_enabled() -> bool {
    env_non_empty(ACTIVITY_LEDGER_TODAY_PATH_ENV).is_some()
        && env_non_empty(ACTIVITY_LEDGER_TODAY_CAPABILITY_ENV).is_some()
}

/// Return the `ToolDef` for `get_activity_ledger_today`.
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
                "includeEvents": {
                    "type": "boolean",
                    "description": "When true, include each journal's normalized events. Defaults to false."
                }
            }
        }),
    }
}

/// Execute a `load_skill` call. Returns a `ToolResult` on success or a
/// user-visible error result if the skill is not found or cannot be read.
pub async fn call_load_skill(arguments: &Value, skills: &[SkillEntry]) -> ToolResult {
    let name = match arguments.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => {
            return error_result("load_skill: missing required argument \"name\"");
        }
    };

    // Two forms:
    //   "skill-name"            → load SKILL.md body + ## Supporting Files section
    //   "skill-name/rel/path"   → load a specific supporting file
    if let Some((skill_name, rel_path)) = name.split_once('/') {
        return load_supporting_file(skill_name, rel_path, skills).await;
    }

    // Plain skill-name form: load SKILL.md body.
    let entry = match skills.iter().find(|s| s.name == name) {
        Some(e) => e,
        None => {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            return error_result(&format!(
                "load_skill: skill {name:?} not found. Available: {available:?}"
            ));
        }
    };

    // Read the file off the async executor to avoid blocking a Tokio worker.
    let skill_path = entry.path.clone();
    let raw = match tokio::task::spawn_blocking(move || std::fs::read_to_string(&skill_path))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
    {
        Ok(s) => s,
        Err(e) => {
            return error_result(&format!("load_skill: could not read {:?}: {e}", entry.path));
        }
    };

    // Strip the YAML frontmatter — the agent already knows name/description
    // from the system prompt; return only the body.
    let body = strip_frontmatter(&raw);

    let mut output = body.to_owned();

    // Append ## Supporting Files section if this skill has any.
    if !entry.supporting_files.is_empty() {
        let skill_dir = entry.path.parent().unwrap_or(&entry.path);
        output.push_str("\n\n## Supporting Files\n\n");
        for file in &entry.supporting_files {
            if let Ok(rel) = file.strip_prefix(skill_dir) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                output.push_str(&format!(
                    "- {} (load_skill(name: \"{}/{}\"))\n",
                    rel_str, entry.name, rel_str
                ));
            }
        }
    }

    // Apply the size cap to the full output (body + Supporting Files section)
    // so the total tool result stays within MAX_SKILL_BODY_BYTES.
    let output = if output.len() > MAX_SKILL_BODY_BYTES {
        truncate_at_boundary(&output, MAX_SKILL_BODY_BYTES).to_owned()
    } else {
        output
    };

    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(output)],
        is_error: false,
    }
}

/// Execute a `get_activity_ledger_today` call against a Desktop-authored local
/// snapshot. Fails closed on missing env, unsafe file properties, schema or
/// capability mismatch, or stale data.
pub async fn call_activity_ledger_today(arguments: &Value) -> ToolResult {
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
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let arguments = arguments.clone();
    match tokio::task::spawn_blocking(move || {
        read_activity_ledger_today(&path, &capability, &arguments, now_secs)
    })
    .await
    .unwrap_or_else(|e| Err(format!("{ACTIVITY_LEDGER_TODAY_TOOL}: task failed: {e}")))
    {
        Ok(output) => ToolResult {
            provider_id: String::new(),
            content: vec![ToolResultContent::Text(output)],
            is_error: false,
        },
        Err(msg) => error_result(&msg),
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

#[derive(Clone)]
struct ActivityLedgerQuery {
    channel_id: Option<String>,
    agent_pubkey: Option<String>,
    status: Option<String>,
    proof_state: Option<String>,
    limit: usize,
    include_events: bool,
}

fn read_activity_ledger_today(
    path: &str,
    capability: &str,
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
    let result = filter_activity_ledger_snapshot(&root, capability, &query, now_secs)?;
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
        include_events: parse_bool_arg(object.get("includeEvents"))?,
    })
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

    let mut filtered = Vec::new();
    for journal in journals {
        if journal_matches_query(journal, query)? {
            filtered.push(strip_events_from_journal(journal, query.include_events)?);
        }
    }
    let matching_journals = filtered.len();
    let truncated = matching_journals > query.limit;
    if truncated {
        filtered.truncate(query.limit);
    }

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
        "generatedAt": generated_at,
        "expiresAt": expires_at,
        "capability": capability,
        "filters": {
            "channelId": query.channel_id,
            "agentPubkey": query.agent_pubkey,
            "status": query.status,
            "proofState": query.proof_state,
            "limit": query.limit,
            "includeEvents": query.include_events,
        },
        "counts": {
            "matchingJournals": matching_journals,
            "returnedJournals": filtered.len(),
            "failed": failed,
            "inProgress": in_progress,
            "claimedWithoutEvidence": claimed_without_evidence,
        },
        "truncated": truncated,
        "journals": filtered,
        "channels": channels,
    }))
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

fn rebuild_filtered_channels(journals: &[Value]) -> Vec<Value> {
    let mut channels: std::collections::BTreeMap<
        String,
        (
            Vec<String>,
            std::collections::BTreeSet<String>,
            std::collections::BTreeSet<String>,
            String,
        ),
    > = std::collections::BTreeMap::new();

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

/// Load a supporting file identified by `skill_name/rel_path`.
/// Matches against the pre-enumerated `supporting_files` list and applies a
/// canonicalize-based traversal guard before reading.
async fn load_supporting_file(
    skill_name: &str,
    rel_path: &str,
    skills: &[SkillEntry],
) -> ToolResult {
    let rel_path = rel_path.replace('\\', "/");

    let entry = match skills.iter().find(|s| s.name == skill_name) {
        Some(e) => e,
        None => {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            return error_result(&format!(
                "load_skill: skill {skill_name:?} not found. Available: {available:?}"
            ));
        }
    };

    let skill_dir = match entry.path.parent() {
        Some(d) => d,
        None => {
            return error_result(&format!(
                "load_skill: could not determine skill directory for {skill_name:?}"
            ));
        }
    };

    // Match rel_path against the pre-enumerated supporting_files list.
    let matched = entry.supporting_files.iter().find(|f| {
        f.strip_prefix(skill_dir)
            .map(|r| r.to_string_lossy().replace('\\', "/") == rel_path)
            .unwrap_or(false)
    });

    let file_path = match matched {
        Some(p) => p,
        None => {
            let available: Vec<String> = entry
                .supporting_files
                .iter()
                .filter_map(|f| {
                    f.strip_prefix(skill_dir)
                        .ok()
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                })
                .collect();
            if available.is_empty() {
                return error_result(&format!(
                    "load_skill: skill {skill_name:?} has no supporting files."
                ));
            }
            return error_result(&format!(
                "load_skill: file {rel_path:?} not found in skill {skill_name:?}. \
                 Available: {available:?}"
            ));
        }
    };

    // Traversal guard: canonicalize both paths and verify the file stays inside
    // the skill directory. Fail hard if the skill directory itself can't be
    // canonicalized — a degraded guard is worse than no guard.
    let canonical_skill_dir = match skill_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return error_result(&format!(
                "load_skill: could not canonicalize skill directory for {skill_name:?}: {e}"
            ));
        }
    };

    // Clone the path so we can move it into spawn_blocking.
    let file_path = file_path.clone();
    let skill_name = skill_name.to_owned();
    let rel_path_owned = rel_path.clone();

    match tokio::task::spawn_blocking(move || file_path.canonicalize().map(|c| (c, file_path)))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
    {
        Ok((canonical_file, resolved_path)) if canonical_file.starts_with(&canonical_skill_dir) => {
            match tokio::task::spawn_blocking(move || std::fs::read_to_string(&resolved_path))
                .await
                .unwrap_or_else(|e| Err(std::io::Error::other(e)))
            {
                Ok(content) => {
                    let output = format!(
                        "# Loaded: {}/{}\n\n{}\n\n---\nFile loaded into context.",
                        skill_name, rel_path_owned, content
                    );
                    let output = if output.len() > MAX_SKILL_BODY_BYTES {
                        truncate_at_boundary(&output, MAX_SKILL_BODY_BYTES).to_owned()
                    } else {
                        output
                    };
                    ToolResult {
                        provider_id: String::new(),
                        content: vec![ToolResultContent::Text(output)],
                        is_error: false,
                    }
                }
                Err(e) => error_result(&format!(
                    "load_skill: could not read {skill_name:?}/{rel_path_owned}: {e}"
                )),
            }
        }
        Ok(_) => error_result(&format!(
            "load_skill: refusing to load {skill_name:?}/{rel_path_owned}: \
             resolves outside the skill directory"
        )),
        Err(e) => error_result(&format!(
            "load_skill: could not resolve {skill_name:?}/{rel_path_owned}: {e}"
        )),
    }
}

fn error_result(msg: &str) -> ToolResult {
    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(msg.to_owned())],
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn text_content(result: &ToolResult) -> String {
        match &result.content[0] {
            ToolResultContent::Text(t) => t.clone(),
            ToolResultContent::Image { .. } => panic!("unexpected Image content in test"),
        }
    }

    fn make_skill(name: &str, description: &str, path: PathBuf) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            path,
            supporting_files: Vec::new(),
        }
    }

    fn make_skill_with_files(
        name: &str,
        description: &str,
        path: PathBuf,
        supporting_files: Vec<PathBuf>,
    ) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            path,
            supporting_files,
        }
    }

    fn write_activity_snapshot(
        dir: &TempDir,
        capability: &str,
        generated_at: u64,
        expires_at: u64,
    ) -> PathBuf {
        let path = dir.path().join("activity-ledger-today.json");
        let snapshot = json!({
            "schema": ACTIVITY_LEDGER_TODAY_SCHEMA,
            "ownerPubkey": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "generatedAt": generated_at,
            "expiresAt": expires_at,
            "capability": capability,
            "surface": {
                "day": "2026-08-21",
                "journals": [
                    {
                        "id": "journal-a",
                        "channelId": "chan-a",
                        "agentPubkey": "agent-a",
                        "agentName": "Honey",
                        "status": "completed",
                        "proofState": "RECEIPTED",
                        "endedAt": "2026-08-21T14:00:00.000Z",
                        "claimedCompletionWithoutEvidence": false,
                        "events": [
                            { "id": "event-a", "detail": "receipted activity" }
                        ]
                    },
                    {
                        "id": "journal-b",
                        "channelId": "chan-b",
                        "agentPubkey": "agent-b",
                        "agentName": "Fizz",
                        "status": "failed",
                        "proofState": "FAILED",
                        "endedAt": "2026-08-21T15:00:00.000Z",
                        "claimedCompletionWithoutEvidence": true,
                        "events": [
                            { "id": "event-b", "detail": "failed activity" }
                        ]
                    }
                ]
            }
        });
        std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    fn write_custom_activity_snapshot(dir: &TempDir, snapshot: Value) -> PathBuf {
        let path = dir.path().join("activity-ledger-today.json");
        std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    #[tokio::test]
    async fn call_load_skill_missing_name_arg() {
        let result = call_load_skill(&serde_json::json!({}), &[]).await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("missing required argument"), "got: {text}");
    }

    #[tokio::test]
    async fn call_load_skill_skill_not_found() {
        let result = call_load_skill(&serde_json::json!({"name": "no-such"}), &[]).await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("not found"), "got: {text}");
    }

    #[tokio::test]
    async fn call_load_skill_returns_body_strips_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let skill_md = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: test\ndescription: A test\n---\nSkill body here.\n",
        )
        .unwrap();
        let skills = vec![make_skill("test", "A test", skill_md)];
        let result = call_load_skill(&serde_json::json!({"name": "test"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(text.contains("Skill body here."), "got: {text}");
        assert!(
            !text.contains("---"),
            "frontmatter should be stripped: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_appends_supporting_files_section() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("foo.md");
        std::fs::write(&ref_file, "Reference content.").unwrap();

        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(&serde_json::json!({"name": "my-skill"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(text.contains("Body."), "body missing: {text}");
        assert!(
            text.contains("## Supporting Files"),
            "missing Supporting Files section: {text}"
        );
        assert!(
            text.contains("references/foo.md"),
            "missing file listing: {text}"
        );
        assert!(
            text.contains("load_skill(name: \"my-skill/references/foo.md\")"),
            "missing load_skill hint: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_no_supporting_files_section_when_empty() {
        let tmp = TempDir::new().unwrap();
        let skill_md = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: bare\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let skills = vec![make_skill("bare", "desc", skill_md)];
        let result = call_load_skill(&serde_json::json!({"name": "bare"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(
            !text.contains("## Supporting Files"),
            "should not have Supporting Files section when none: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_supporting_file_returns_content() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("foo.md");
        std::fs::write(&ref_file, "Reference content here.").unwrap();

        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(
            &serde_json::json!({"name": "my-skill/references/foo.md"}),
            &skills,
        )
        .await;
        assert!(!result.is_error, "expected success, got error");
        let text = text_content(&result);
        assert!(
            text.contains("Reference content here."),
            "file content missing: {text}"
        );
        assert!(
            text.contains("# Loaded: my-skill/references/foo.md"),
            "missing header: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_supporting_file_not_found_lists_available() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("foo.md");
        std::fs::write(&ref_file, "content").unwrap();

        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(
            &serde_json::json!({"name": "my-skill/references/missing.md"}),
            &skills,
        )
        .await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("not found"), "got: {text}");
        assert!(
            text.contains("references/foo.md"),
            "should list available: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_no_supporting_files_error_message() {
        let tmp = TempDir::new().unwrap();
        let skill_md = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: bare\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let skills = vec![make_skill("bare", "desc", skill_md)];
        let result =
            call_load_skill(&serde_json::json!({"name": "bare/anything.md"}), &skills).await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("no supporting files"), "got: {text}");
    }

    #[tokio::test]
    async fn call_load_skill_traversal_guard_rejects_escape() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();

        // Create a file outside the skill dir that we'll try to reference.
        let outside_file = tmp.path().join("secret.txt");
        std::fs::write(&outside_file, "secret content").unwrap();

        // Manually construct a SkillEntry with a supporting_files entry that
        // points outside the skill dir — simulating a crafted/malicious entry.
        // The traversal guard should catch this.
        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md.clone(),
            vec![outside_file.clone()],
        )];

        // The slash form splits "my-skill/../secret.txt" into skill_name="my-skill"
        // and rel_path="../secret.txt". strip_prefix(skill_dir) on outside_file
        // fails, so it won't match any supporting_files entry — the pre-enumeration
        // guard rejects it before the canonicalize guard even fires.
        let result = call_load_skill(
            &serde_json::json!({"name": "my-skill/../secret.txt"}),
            &skills,
        )
        .await;
        assert!(result.is_error, "traversal attempt should be rejected");
        let text = text_content(&result);
        assert!(
            !text.contains("secret content"),
            "secret content must not be returned: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_truncates_large_body() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        // Build a body that exceeds MAX_SKILL_BODY_BYTES (32 KiB).
        let large_body = "x".repeat(40 * 1024);
        std::fs::write(
            &skill_md,
            format!("---\nname: big\ndescription: desc\n---\n{large_body}\n"),
        )
        .unwrap();
        // Add a supporting file so the Supporting Files section is also appended
        // before the cap is applied.
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("extra.md");
        std::fs::write(&ref_file, "extra content").unwrap();

        let skills = vec![make_skill_with_files(
            "big",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(&serde_json::json!({"name": "big"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(
            text.len() <= MAX_SKILL_BODY_BYTES,
            "output length {} exceeds MAX_SKILL_BODY_BYTES {}",
            text.len(),
            MAX_SKILL_BODY_BYTES
        );
    }

    #[tokio::test]
    async fn call_load_skill_truncates_large_supporting_file() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(&skill_md, "---\nname: big\ndescription: desc\n---\nBody.\n").unwrap();

        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("huge.md");
        std::fs::write(&ref_file, "x".repeat(MAX_SKILL_BODY_BYTES * 2)).unwrap();

        let skills = vec![make_skill_with_files(
            "big",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(
            &serde_json::json!({"name": "big/references/huge.md"}),
            &skills,
        )
        .await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(
            text.len() <= MAX_SKILL_BODY_BYTES,
            "output length {} exceeds MAX_SKILL_BODY_BYTES {}",
            text.len(),
            MAX_SKILL_BODY_BYTES
        );
        assert!(
            text.starts_with("# Loaded: big/references/huge.md"),
            "missing supporting-file header: {text}"
        );
    }

    #[test]
    fn activity_ledger_today_filters_and_strips_events_by_default() {
        let tmp = TempDir::new().unwrap();
        let capability = "buzz.activity-ledger.today.read/v1";
        let path = write_activity_snapshot(&tmp, capability, 100, 160);

        let output = read_activity_ledger_today(
            path.to_str().unwrap(),
            capability,
            &json!({"agentPubkey": "agent-a", "limit": 10}),
            120,
        )
        .unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["counts"]["matchingJournals"], 1);
        assert_eq!(result["counts"]["returnedJournals"], 1);
        assert_eq!(result["journals"][0]["id"], "journal-a");
        assert!(result["journals"][0].get("events").is_none());
        assert_eq!(result["channels"][0]["channelId"], "chan-a");
    }

    #[test]
    fn activity_ledger_today_includes_events_when_requested() {
        let tmp = TempDir::new().unwrap();
        let capability = "buzz.activity-ledger.today.read/v1";
        let path = write_activity_snapshot(&tmp, capability, 100, 160);

        let output = read_activity_ledger_today(
            path.to_str().unwrap(),
            capability,
            &json!({"channelId": "chan-b", "includeEvents": true, "limit": 10}),
            120,
        )
        .unwrap();
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["journals"][0]["id"], "journal-b");
        assert_eq!(result["journals"][0]["events"][0]["id"], "event-b");
        assert_eq!(result["counts"]["failed"], 1);
        assert_eq!(result["counts"]["claimedWithoutEvidence"], 1);
    }

    #[test]
    fn activity_ledger_today_rejects_relative_path() {
        let error = read_activity_ledger_today(
            "relative.json",
            "buzz.activity-ledger.today.read/v1",
            &json!({}),
            120,
        )
        .unwrap_err();
        assert!(
            error.contains("snapshot path must be absolute"),
            "got: {error}"
        );
    }

    #[test]
    fn activity_ledger_today_rejects_oversized_snapshot_before_reading() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("oversized.json");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES + 1)
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let error = read_activity_ledger_today(
            path.to_str().unwrap(),
            "buzz.activity-ledger.today.read/v1",
            &json!({}),
            120,
        )
        .unwrap_err();
        assert!(error.contains("snapshot exceeds"), "got: {error}");
    }

    #[test]
    fn activity_ledger_today_rejects_capability_mismatch_and_staleness() {
        let tmp = TempDir::new().unwrap();
        let capability = "buzz.activity-ledger.today.read/v1";
        let path = write_activity_snapshot(&tmp, "wrong-capability", 100, 160);

        let capability_error =
            read_activity_ledger_today(path.to_str().unwrap(), capability, &json!({}), 120)
                .unwrap_err();
        assert!(
            capability_error.contains("capability mismatch"),
            "got: {capability_error}"
        );

        let stale_path = write_activity_snapshot(&tmp, capability, 100, 110);
        let stale_error =
            read_activity_ledger_today(stale_path.to_str().unwrap(), capability, &json!({}), 120)
                .unwrap_err();
        assert!(
            stale_error.contains("snapshot expired"),
            "got: {stale_error}"
        );
    }

    #[test]
    fn activity_ledger_today_rejects_future_generated_at_and_uppercase_owner() {
        let tmp = TempDir::new().unwrap();
        let capability = "buzz.activity-ledger.today.read/v1";

        let future_path = write_activity_snapshot(&tmp, capability, 500, 560);
        let future_error =
            read_activity_ledger_today(future_path.to_str().unwrap(), capability, &json!({}), 120)
                .unwrap_err();
        assert!(
            future_error.contains("generatedAt is more than 300 seconds in the future"),
            "got: {future_error}"
        );

        let uppercase_owner_path = write_custom_activity_snapshot(
            &tmp,
            json!({
                "schema": ACTIVITY_LEDGER_TODAY_SCHEMA,
                "ownerPubkey": "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD",
                "generatedAt": 100,
                "expiresAt": 160,
                "capability": capability,
                "surface": {
                    "day": "2026-08-21",
                    "journals": []
                }
            }),
        );
        let owner_error = read_activity_ledger_today(
            uppercase_owner_path.to_str().unwrap(),
            capability,
            &json!({}),
            120,
        )
        .unwrap_err();
        assert!(
            owner_error.contains("ownerPubkey must be 64 lowercase hex chars"),
            "got: {owner_error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn activity_ledger_today_rejects_symlink_and_non_0600_mode() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let tmp = TempDir::new().unwrap();
        let capability = "buzz.activity-ledger.today.read/v1";
        let path = write_activity_snapshot(&tmp, capability, 100, 160);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mode_error =
            read_activity_ledger_today(path.to_str().unwrap(), capability, &json!({}), 120)
                .unwrap_err();
        assert!(
            mode_error.contains("snapshot mode must be 0600"),
            "got: {mode_error}"
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link_path = tmp.path().join("linked.json");
        symlink(&path, &link_path).unwrap();
        let symlink_error =
            read_activity_ledger_today(link_path.to_str().unwrap(), capability, &json!({}), 120)
                .unwrap_err();
        assert!(
            symlink_error.contains("must not be a symlink")
                || symlink_error.contains("could not open snapshot"),
            "got: {symlink_error}"
        );
    }
}
