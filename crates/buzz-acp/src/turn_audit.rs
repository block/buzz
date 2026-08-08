//! Bounded, content-free, durable audit trail for inbound agent turns.
//!
//! The relay observer carries rich ACP payloads for the Desktop UI. This
//! module consumes that in-process feed but persists only a fixed metadata
//! schema: event/turn IDs, channel IDs, stage timestamps, outcome labels, and
//! relay publish results. Message bodies, prompts, tool inputs, credentials,
//! and raw ACP frames are never serialized.

use std::{
    collections::{HashMap, VecDeque},
    fs::{self},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::observer::{ObserverEvent, ObserverHandle};

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditFile {
    schema_version: u32,
    records: VecDeque<TurnAuditRecord>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    gaps: VecDeque<AuditGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditGap {
    detected_at: String,
    skipped_events: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnAuditRecord {
    correlation_id: String,
    channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<String>,
    received_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    admission: Option<Admission>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queued_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    acp_submitted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_output_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion: Option<Completion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    publish_attempts: Vec<PublishAttempt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Admission {
    at: String,
    status: AdmissionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AdmissionStatus {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Completion {
    at: String,
    outcome: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishAttempt {
    attempted_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accepted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<String>,
}

struct AuditWriter {
    path: PathBuf,
    retention: usize,
    file: AuditFile,
    turn_events: HashMap<String, Vec<String>>,
    pending_publish_tools: HashMap<(String, String), Vec<(String, usize)>>,
}

impl AuditWriter {
    fn open(path: PathBuf, retention: usize) -> Self {
        let file = load_file(&path, retention).unwrap_or_else(|error| {
            preserve_corrupt_file(&path, &error);
            AuditFile {
                schema_version: SCHEMA_VERSION,
                records: VecDeque::new(),
                gaps: VecDeque::new(),
            }
        });
        let mut turn_events: HashMap<String, Vec<String>> = HashMap::new();
        for record in &file.records {
            if let Some(turn_id) = &record.turn_id {
                turn_events
                    .entry(turn_id.clone())
                    .or_default()
                    .push(record.correlation_id.clone());
            }
        }
        Self {
            path,
            retention: retention.max(1),
            file,
            turn_events,
            pending_publish_tools: HashMap::new(),
        }
    }

    fn record_gap(&mut self, skipped_events: u64) {
        self.file.gaps.push_back(AuditGap {
            detected_at: chrono::Utc::now().to_rfc3339(),
            skipped_events,
        });
        while self.file.gaps.len() > 100 {
            self.file.gaps.pop_front();
        }
        if let Err(error) = persist_atomic(&self.path, &self.file) {
            tracing::warn!(target: "turn_audit", path = %self.path.display(), "failed to persist turn audit gap: {error}");
        }
    }

    fn ingest(&mut self, event: ObserverEvent) {
        let changed = match event.kind.as_str() {
            "turn_received" => self.received(&event),
            "turn_rejected" => self.rejected(&event),
            "turn_queued" => self.queued(&event),
            "turn_started" => self.started(&event),
            "acp_submitted" => self.for_turn(&event, |record, timestamp| {
                set_once(&mut record.acp_submitted_at, timestamp)
            }),
            "acp_read" => self.acp_read(&event),
            "turn_outcome" => self.for_turn(&event, |record, timestamp| {
                let Some(outcome) = event.payload.get("outcome").and_then(|v| v.as_str()) else {
                    return false;
                };
                if record.completion.is_some() {
                    return false;
                }
                record.completion = Some(Completion {
                    at: timestamp.to_string(),
                    outcome: outcome.to_string(),
                });
                true
            }),
            "turn_error" => self.for_turn(&event, |record, timestamp| {
                if record.completion.is_some() {
                    return false;
                }
                record.completion = Some(Completion {
                    at: timestamp.to_string(),
                    outcome: "error".to_string(),
                });
                true
            }),
            _ => false,
        };

        if changed {
            self.enforce_retention();
            if let Err(error) = persist_atomic(&self.path, &self.file) {
                tracing::warn!(
                    target: "turn_audit",
                    path = %self.path.display(),
                    "failed to persist turn audit: {error}"
                );
            }
        }
    }

    fn received(&mut self, event: &ObserverEvent) -> bool {
        let Some(event_id) = payload_str(&event.payload, "eventId") else {
            return false;
        };
        if self.record(event_id).is_some() {
            return false;
        }
        let Some(channel_id) = event.channel_id.clone() else {
            return false;
        };
        self.file.records.push_back(TurnAuditRecord {
            correlation_id: event_id.to_string(),
            channel_id,
            turn_id: None,
            received_at: event.timestamp.clone(),
            admission: None,
            queued_at: None,
            acp_submitted_at: None,
            first_output_at: None,
            completion: None,
            publish_attempts: Vec::new(),
        });
        true
    }

    fn rejected(&mut self, event: &ObserverEvent) -> bool {
        let Some(event_id) = payload_str(&event.payload, "eventId") else {
            return false;
        };
        let reason = payload_str(&event.payload, "reason").unwrap_or("unspecified");
        let Some(record) = self.record_mut(event_id) else {
            return false;
        };
        record.admission = Some(Admission {
            at: event.timestamp.clone(),
            status: AdmissionStatus::Rejected,
            reason: Some(reason.to_string()),
        });
        record.completion = Some(Completion {
            at: event.timestamp.clone(),
            outcome: "rejected".to_string(),
        });
        true
    }

    fn queued(&mut self, event: &ObserverEvent) -> bool {
        let Some(event_id) = payload_str(&event.payload, "eventId") else {
            return false;
        };
        let Some(record) = self.record_mut(event_id) else {
            return false;
        };
        if record.queued_at.is_some() {
            return false;
        }
        record.admission = Some(Admission {
            at: event.timestamp.clone(),
            status: AdmissionStatus::Accepted,
            reason: None,
        });
        record.queued_at = Some(event.timestamp.clone());
        true
    }

    fn started(&mut self, event: &ObserverEvent) -> bool {
        let Some(turn_id) = event.turn_id.as_deref() else {
            return false;
        };
        let event_ids = payload_string_array(&event.payload, "triggeringEventIds");
        if event_ids.is_empty() {
            return false;
        }
        self.turn_events
            .insert(turn_id.to_string(), event_ids.clone());
        let mut changed = false;
        for event_id in event_ids {
            if let Some(record) = self.record_mut(&event_id) {
                if record.turn_id.as_deref() != Some(turn_id) {
                    record.turn_id = Some(turn_id.to_string());
                    changed = true;
                }
            }
        }
        changed
    }

    fn acp_read(&mut self, event: &ObserverEvent) -> bool {
        let Some(turn_id) = event.turn_id.as_deref() else {
            return false;
        };
        let mut changed = false;
        if is_first_output(&event.payload) {
            changed |= self.for_turn(event, |record, timestamp| {
                set_once(&mut record.first_output_at, timestamp)
            });
        }

        let Some(update) = event.payload.get("params").and_then(|v| v.get("update")) else {
            return changed;
        };
        let update_type = update.get("sessionUpdate").and_then(|v| v.as_str());
        let tool_id = update.get("toolCallId").and_then(|v| v.as_str());
        match (update_type, tool_id) {
            (Some("tool_call"), Some(tool_id)) if contains_buzz_message_send(update) => {
                let event_ids = self.turn_events.get(turn_id).cloned().unwrap_or_default();
                if !event_ids.is_empty() {
                    let mut attempts = Vec::with_capacity(event_ids.len());
                    for event_id in event_ids {
                        if let Some(record) = self.record_mut(&event_id) {
                            let attempt_index = record.publish_attempts.len();
                            record.publish_attempts.push(PublishAttempt {
                                attempted_at: event.timestamp.clone(),
                                completed_at: None,
                                accepted: None,
                                reply_event_id: None,
                                failure: None,
                            });
                            attempts.push((event_id, attempt_index));
                            changed = true;
                        }
                    }
                    self.pending_publish_tools
                        .insert((turn_id.to_string(), tool_id.to_string()), attempts);
                }
            }
            (Some("tool_call_update"), Some(tool_id)) => {
                let key = (turn_id.to_string(), tool_id.to_string());
                if let Some(attempts) = self.pending_publish_tools.remove(&key) {
                    let status = update.get("status").and_then(|v| v.as_str());
                    let publish_result = extract_publish_result(update);
                    for (event_id, attempt_index) in attempts {
                        let Some(record) = self.record_mut(&event_id) else {
                            continue;
                        };
                        let Some(attempt) = record.publish_attempts.get_mut(attempt_index) else {
                            continue;
                        };
                        attempt.completed_at = Some(event.timestamp.clone());
                        if let Some((event_id, accepted)) = &publish_result {
                            attempt.reply_event_id = Some(event_id.clone());
                            attempt.accepted = Some(*accepted);
                            if !accepted {
                                attempt.failure = Some("relay_rejected".to_string());
                            }
                        } else if status == Some("failed") {
                            attempt.failure = Some("publish_failed".to_string());
                        } else {
                            attempt.failure = Some("result_unavailable".to_string());
                        }
                        changed = true;
                    }
                }
            }
            _ => {}
        }
        changed
    }

    fn for_turn(
        &mut self,
        event: &ObserverEvent,
        mut update: impl FnMut(&mut TurnAuditRecord, &str) -> bool,
    ) -> bool {
        let Some(turn_id) = event.turn_id.as_deref() else {
            return false;
        };
        let event_ids = self.turn_events.get(turn_id).cloned().unwrap_or_default();
        let mut changed = false;
        for event_id in event_ids {
            if let Some(record) = self.record_mut(&event_id) {
                changed |= update(record, &event.timestamp);
            }
        }
        changed
    }

    fn record(&self, event_id: &str) -> Option<&TurnAuditRecord> {
        self.file
            .records
            .iter()
            .find(|record| record.correlation_id == event_id)
    }

    fn record_mut(&mut self, event_id: &str) -> Option<&mut TurnAuditRecord> {
        self.file
            .records
            .iter_mut()
            .find(|record| record.correlation_id == event_id)
    }

    fn enforce_retention(&mut self) {
        while self.file.records.len() > self.retention {
            let terminal = self
                .file
                .records
                .iter()
                .position(|record| record.completion.is_some());
            let index = terminal.unwrap_or(0);
            if let Some(removed) = self.file.records.remove(index) {
                if let Some(turn_id) = removed.turn_id {
                    if let Some(ids) = self.turn_events.get_mut(&turn_id) {
                        ids.retain(|id| id != &removed.correlation_id);
                        if ids.is_empty() {
                            self.turn_events.remove(&turn_id);
                        }
                    }
                }
            }
        }
    }
}

/// Start the local audit consumer. The task owns all file mutation so stage
/// order is deterministic and no lock is held on the hot event path.
pub struct TurnAuditTask {
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl TurnAuditTask {
    /// Stop accepting new frames, persist everything already queued on the
    /// observer bus, and wait for the audit writer to finish.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        if let Err(error) = self.handle.await {
            tracing::warn!(target: "turn_audit", "turn audit task failed during shutdown: {error}");
        }
    }
}

pub fn spawn(observer: &ObserverHandle, path: PathBuf, retention: usize) -> TurnAuditTask {
    let mut receiver = observer.subscribe();
    let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let mut writer = AuditWriter::open(path, retention);
        loop {
            tokio::select! {
                biased;
                result = receiver.recv() => match result {
                    Ok(event) => writer.ingest(event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(target: "turn_audit", skipped, "turn audit consumer lagged");
                        writer.record_gap(skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = &mut shutdown_rx => {
                    while let Ok(event) = receiver.try_recv() {
                        writer.ingest(event);
                    }
                    break;
                }
            }
        }
    });
    TurnAuditTask { shutdown, handle }
}

/// Stable per-agent/per-relay audit path without exposing the relay URL.
pub fn audit_path(base_dir: &Path, agent_pubkey: &str, relay_url: &str) -> PathBuf {
    let digest = Sha256::digest(relay_url.as_bytes());
    base_dir.join(format!(
        "turn-audit-{}-{}.json",
        &agent_pubkey[..agent_pubkey.len().min(16)],
        hex::encode(&digest[..8])
    ))
}

fn load_file(path: &Path, retention: usize) -> io::Result<AuditFile> {
    if !path.exists() {
        return Ok(AuditFile {
            schema_version: SCHEMA_VERSION,
            records: VecDeque::new(),
            gaps: VecDeque::new(),
        });
    }
    let bytes = fs::read(path)?;
    let mut file: AuditFile = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if file.schema_version != SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported schema version {}", file.schema_version),
        ));
    }
    while file.records.len() > retention.max(1) {
        file.records.pop_front();
    }
    Ok(file)
}

fn persist_atomic(path: &Path, file: &AuditFile) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "audit path has no parent",
        ));
    };
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut output = atomic_write_file::AtomicWriteFile::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer_pretty(&mut output, file).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.commit()
}

fn preserve_corrupt_file(path: &Path, error: &io::Error) {
    if !path.exists() {
        return;
    }
    let suffix = chrono::Utc::now().format("%Y%m%dT%H%M%S%.fZ");
    let backup = path.with_extension(format!("corrupt-{suffix}.json"));
    match fs::rename(path, &backup) {
        Ok(()) => tracing::warn!(
            target: "turn_audit",
            path = %path.display(),
            backup = %backup.display(),
            "preserved unreadable turn audit and started clean: {error}"
        ),
        Err(rename_error) => tracing::warn!(
            target: "turn_audit",
            path = %path.display(),
            "turn audit is unreadable ({error}) and could not be preserved: {rename_error}"
        ),
    }
}

fn payload_str<'a>(payload: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    payload.get(key)?.as_str()
}

fn payload_string_array(payload: &serde_json::Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

fn set_once(target: &mut Option<String>, value: &str) -> bool {
    if target.is_some() {
        false
    } else {
        *target = Some(value.to_string());
        true
    }
}

fn is_first_output(payload: &serde_json::Value) -> bool {
    let update_type = payload
        .get("params")
        .and_then(|value| value.get("update"))
        .and_then(|value| value.get("sessionUpdate"))
        .and_then(|value| value.as_str());
    matches!(
        update_type,
        Some("agent_message_chunk" | "agent_thought_chunk" | "tool_call" | "plan")
    )
}

fn contains_buzz_message_send(value: &serde_json::Value) -> bool {
    fn visit(value: &serde_json::Value, strings: &mut Vec<String>) {
        match value {
            serde_json::Value::String(value) => strings.push(value.to_ascii_lowercase()),
            serde_json::Value::Array(values) => {
                for value in values {
                    visit(value, strings);
                }
            }
            serde_json::Value::Object(values) => {
                for value in values.values() {
                    visit(value, strings);
                }
            }
            _ => {}
        }
    }
    let mut strings = Vec::new();
    visit(value, &mut strings);
    let joined = strings.join(" ");
    joined.contains("buzz messages send")
        || joined.contains("buzz\",\"messages\",\"send")
        || joined.contains("buzz', 'messages', 'send")
}

fn extract_publish_result(value: &serde_json::Value) -> Option<(String, bool)> {
    match value {
        serde_json::Value::Object(map) => {
            let event_id = map
                .get("event_id")
                .or_else(|| map.get("eventId"))
                .and_then(|value| value.as_str());
            let accepted = map.get("accepted").and_then(|value| value.as_bool());
            if let (Some(event_id), Some(accepted)) = (event_id, accepted) {
                if event_id.len() == 64 && event_id.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some((event_id.to_ascii_lowercase(), accepted));
                }
            }
            map.values().find_map(extract_publish_result)
        }
        serde_json::Value::Array(values) => values.iter().find_map(extract_publish_result),
        serde_json::Value::String(text) => extract_publish_result_from_text(text),
        _ => None,
    }
}

fn extract_publish_result_from_text(text: &str) -> Option<(String, bool)> {
    if let Ok(parsed @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) =
        serde_json::from_str::<serde_json::Value>(text.trim())
    {
        if let Some(result) = extract_publish_result(&parsed) {
            return Some(result);
        }
    }
    for line in text.lines() {
        for (attempt, (offset, _)) in line.match_indices('{').enumerate() {
            if attempt >= 32 {
                break;
            }
            let mut values = serde_json::Deserializer::from_str(&line[offset..])
                .into_iter::<serde_json::Value>();
            if let Some(Ok(parsed)) = values.next() {
                if let Some(result) = extract_publish_result(&parsed) {
                    return Some(result);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{context_for, ObserverEvent};

    fn event(
        seq: u64,
        kind: &str,
        event_id: &str,
        turn_id: Option<&str>,
        payload: serde_json::Value,
    ) -> ObserverEvent {
        ObserverEvent {
            seq,
            timestamp: format!("2026-08-07T12:00:{seq:02}Z"),
            kind: kind.to_string(),
            agent_index: Some(0),
            channel_id: Some("4c0769da-da97-4e9d-bf12-152ddbaec0aa".to_string()),
            session_id: Some("session-secret-not-written".to_string()),
            turn_id: turn_id.map(str::to_string),
            started_at: None,
            payload: if payload.is_null() {
                serde_json::json!({"eventId": event_id})
            } else {
                payload
            },
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("buzz-turn-audit-{name}-{}", uuid::Uuid::new_v4()))
            .join("audit.json")
    }

    #[test]
    fn persists_complete_metadata_without_raw_content() {
        let path = temp_path("complete");
        let event_id = "a".repeat(64);
        let reply_id = "b".repeat(64);
        let mut writer = AuditWriter::open(path.clone(), 100);
        writer.ingest(event(
            1,
            "turn_received",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        writer.ingest(event(
            2,
            "turn_queued",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        writer.ingest(event(
            3,
            "turn_started",
            &event_id,
            Some("turn-1"),
            serde_json::json!({"triggeringEventIds": [event_id]}),
        ));
        writer.ingest(event(
            4,
            "acp_submitted",
            "",
            Some("turn-1"),
            serde_json::json!({}),
        ));
        writer.ingest(event(
            5,
            "acp_read",
            "",
            Some("turn-1"),
            serde_json::json!({"params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tool-1","rawInput":{"cmd":"buzz messages send --content secret-prompt"}}}}),
        ));
        writer.ingest(event(
            6,
            "acp_read",
            "",
            Some("turn-1"),
            serde_json::json!({"params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"completed","content":[{"text": serde_json::json!({"accepted":true,"event_id":reply_id,"message":"private output"}).to_string()}]}}}),
        ));
        writer.ingest(event(
            7,
            "turn_outcome",
            "",
            Some("turn-1"),
            serde_json::json!({"outcome":"ok","error":"must not persist"}),
        ));

        let bytes = fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("secret-prompt"));
        assert!(!bytes.contains("private output"));
        assert!(!bytes.contains("session-secret"));
        assert!(bytes.contains(&reply_id));
        let file: AuditFile = serde_json::from_str(&bytes).unwrap();
        let record = &file.records[0];
        assert_eq!(record.turn_id.as_deref(), Some("turn-1"));
        assert!(record.first_output_at.is_some());
        assert_eq!(record.publish_attempts[0].accepted, Some(true));
        assert_eq!(record.completion.as_ref().unwrap().outcome, "ok");
    }

    #[test]
    fn rejection_and_publish_failure_paths_are_terminal_and_bounded() {
        let path = temp_path("failures");
        let mut writer = AuditWriter::open(path.clone(), 2);
        for index in 0..3 {
            let event_id = format!("{index:064x}");
            writer.ingest(event(
                1,
                "turn_received",
                &event_id,
                None,
                serde_json::Value::Null,
            ));
            writer.ingest(event(
                2,
                "turn_rejected",
                &event_id,
                None,
                serde_json::json!({"eventId":event_id,"reason":"mention_required"}),
            ));
        }
        let file: AuditFile = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(file.records.len(), 2);
        assert!(file
            .records
            .iter()
            .all(|record| record.completion.is_some()));
    }

    #[test]
    fn restart_loads_existing_records_and_corruption_is_preserved() {
        let path = temp_path("restart");
        let event_id = "c".repeat(64);
        let mut first = AuditWriter::open(path.clone(), 10);
        first.ingest(event(
            1,
            "turn_received",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        drop(first);
        let second = AuditWriter::open(path.clone(), 10);
        assert!(second.record(&event_id).is_some());
        fs::write(&path, b"{truncated").unwrap();
        let third = AuditWriter::open(path.clone(), 10);
        assert!(third.file.records.is_empty());
        let parent = path.parent().unwrap();
        assert!(fs::read_dir(parent)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains("corrupt-")));
    }

    #[test]
    fn relay_rejection_result_is_recorded_without_relay_message() {
        let value = serde_json::json!({
            "content": [{"text": format!(r#"{{"accepted":false,"event_id":"{}","message":"secret rejection detail"}}"#, "d".repeat(64))}]
        });
        assert_eq!(
            extract_publish_result(&value),
            Some(("d".repeat(64), false))
        );
    }

    #[test]
    fn extracts_buzz_result_from_exec_style_wrapper_text() {
        let reply_id = "6".repeat(64);
        let value = serde_json::json!({
            "output": format!("Script completed\nOutput:\n{{\"accepted\":true,\"event_id\":\"{reply_id}\",\"message\":\"private\"}}\n")
        });
        assert_eq!(extract_publish_result(&value), Some((reply_id, true)));
    }

    #[test]
    fn overlapping_publish_results_update_their_own_attempts() {
        let path = temp_path("overlap");
        let inbound_id = "5".repeat(64);
        let reply_a = "a".repeat(64);
        let reply_b = "b".repeat(64);
        let mut writer = AuditWriter::open(path.clone(), 10);
        for stage in ["turn_received", "turn_queued"] {
            writer.ingest(event(1, stage, &inbound_id, None, serde_json::Value::Null));
        }
        writer.ingest(event(
            2,
            "turn_started",
            &inbound_id,
            Some("turn-overlap"),
            serde_json::json!({"triggeringEventIds": [inbound_id]}),
        ));
        for tool_id in ["tool-a", "tool-b"] {
            writer.ingest(event(
                3,
                "acp_read",
                "",
                Some("turn-overlap"),
                serde_json::json!({"params":{"update":{"sessionUpdate":"tool_call","toolCallId":tool_id,"rawInput":{"cmd":"buzz messages send"}}}}),
            ));
        }
        for (tool_id, reply_id) in [("tool-a", &reply_a), ("tool-b", &reply_b)] {
            writer.ingest(event(
                4,
                "acp_read",
                "",
                Some("turn-overlap"),
                serde_json::json!({"params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":tool_id,"status":"completed","content":[{"text":serde_json::json!({"accepted":true,"event_id":reply_id}).to_string()}]}}}),
            ));
        }
        let file: AuditFile = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            file.records[0].publish_attempts[0]
                .reply_event_id
                .as_deref(),
            Some(reply_a.as_str())
        );
        assert_eq!(
            file.records[0].publish_attempts[1]
                .reply_event_id
                .as_deref(),
            Some(reply_b.as_str())
        );
    }

    #[test]
    fn observer_lag_is_durably_marked() {
        let path = temp_path("gap");
        let mut writer = AuditWriter::open(path.clone(), 10);
        writer.record_gap(17);
        let file: AuditFile = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(file.gaps.len(), 1);
        assert_eq!(file.gaps[0].skipped_events, 17);
    }

    #[test]
    fn failed_publish_attempt_is_recorded_without_raw_error() {
        let path = temp_path("publish-failed");
        let event_id = "f".repeat(64);
        let mut writer = AuditWriter::open(path.clone(), 10);
        writer.ingest(event(
            1,
            "turn_received",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        writer.ingest(event(
            2,
            "turn_queued",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        writer.ingest(event(
            3,
            "turn_started",
            &event_id,
            Some("turn-failed"),
            serde_json::json!({"triggeringEventIds": [event_id]}),
        ));
        writer.ingest(event(
            4,
            "acp_read",
            "",
            Some("turn-failed"),
            serde_json::json!({"params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tool-failed","rawInput":{"cmd":"buzz messages send"}}}}),
        ));
        writer.ingest(event(
            5,
            "acp_read",
            "",
            Some("turn-failed"),
            serde_json::json!({"params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tool-failed","status":"failed","rawOutput":{"error":"credential value must never persist"}}}}),
        ));

        let bytes = fs::read_to_string(path).unwrap();
        assert!(!bytes.contains("credential value"));
        let file: AuditFile = serde_json::from_str(&bytes).unwrap();
        assert_eq!(
            file.records[0].publish_attempts[0].failure.as_deref(),
            Some("publish_failed")
        );
    }

    #[test]
    fn audit_path_does_not_expose_relay_url() {
        let path = audit_path(
            Path::new("/tmp/state"),
            &"e".repeat(64),
            "wss://private-relay.example/tenant-secret",
        );
        let rendered = path.to_string_lossy();
        assert!(!rendered.contains("private-relay"));
        assert!(!rendered.contains("tenant-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn persisted_audit_is_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_path("permissions");
        let event_id = "7".repeat(64);
        let mut writer = AuditWriter::open(path.clone(), 10);
        writer.ingest(event(
            1,
            "turn_received",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn terminal_turn_error_is_persisted_without_error_detail() {
        let path = temp_path("turn-error");
        let event_id = "9".repeat(64);
        let mut writer = AuditWriter::open(path.clone(), 10);
        writer.ingest(event(
            1,
            "turn_received",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        writer.ingest(event(
            2,
            "turn_queued",
            &event_id,
            None,
            serde_json::Value::Null,
        ));
        writer.ingest(event(
            3,
            "turn_started",
            &event_id,
            Some("turn-error"),
            serde_json::json!({"triggeringEventIds": [event_id]}),
        ));
        writer.ingest(event(
            4,
            "turn_error",
            "",
            Some("turn-error"),
            serde_json::json!({"error":"secret provider failure"}),
        ));

        let bytes = fs::read_to_string(path).unwrap();
        assert!(!bytes.contains("secret provider failure"));
        let file: AuditFile = serde_json::from_str(&bytes).unwrap();
        assert_eq!(
            file.records[0].completion.as_ref().unwrap().outcome,
            "error"
        );
    }

    #[tokio::test]
    async fn shutdown_drains_queued_observer_frames() {
        let path = temp_path("shutdown-drain");
        let observer = ObserverHandle::in_process_unbuffered();
        let task = spawn(&observer, path.clone(), 10);
        let context = context_for(
            Some(uuid::Uuid::parse_str("4c0769da-da97-4e9d-bf12-152ddbaec0aa").unwrap()),
            None,
            Some("turn-drain".to_string()),
        );
        let event_id = "8".repeat(64);
        observer.emit(
            "turn_received",
            Some(0),
            &context,
            serde_json::json!({"eventId": event_id}),
        );
        observer.emit(
            "turn_queued",
            Some(0),
            &context,
            serde_json::json!({"eventId": event_id}),
        );
        task.shutdown().await;

        let file: AuditFile = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(file.records.len(), 1);
        assert!(file.records[0].queued_at.is_some());
    }

    #[test]
    fn observer_context_helper_remains_content_free() {
        let context = context_for(None, None, Some("turn-1".to_string()));
        assert_eq!(context.turn_id.as_deref(), Some("turn-1"));
    }
}
