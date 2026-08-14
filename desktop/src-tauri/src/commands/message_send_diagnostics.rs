use std::{
    fs::{self, OpenOptions},
    future::Future,
    io::Write as _,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const MAX_LOG_SIZE: u64 = 2 * 1024 * 1024;
static MESSAGE_SEND_LOG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendDiagnosticEntry {
    pub operation_id: String,
    pub stage: String,
    pub transport: String,
    pub channel_id: Option<String>,
    pub event_id: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub wait_ms: Option<u64>,
    pub gate_remaining_ms: Option<u64>,
    pub connection_state: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Serialize)]
struct StoredMessageSendDiagnostic<'a> {
    timestamp_ms: u128,
    #[serde(flatten)]
    entry: &'a MessageSendDiagnosticEntry,
}

fn message_send_log_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join("diagnostics");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create diagnostics dir: {error}"))?;
    Ok(dir.join("message-send.jsonl"))
}

fn valid_token(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

fn validate_entry(entry: &MessageSendDiagnosticEntry) -> Result<(), String> {
    for (label, value, max_len) in [
        ("operation_id", entry.operation_id.as_str(), 64),
        ("stage", entry.stage.as_str(), 64),
        ("transport", entry.transport.as_str(), 24),
    ] {
        if !valid_token(value, max_len) {
            return Err(format!("invalid message-send diagnostic {label}"));
        }
    }

    for (label, value, max_len) in [
        ("channel_id", entry.channel_id.as_deref(), 64),
        ("event_id", entry.event_id.as_deref(), 128),
        ("connection_state", entry.connection_state.as_deref(), 32),
        ("outcome", entry.outcome.as_deref(), 64),
    ] {
        if value.is_some_and(|value| !valid_token(value, max_len)) {
            return Err(format!("invalid message-send diagnostic {label}"));
        }
    }
    Ok(())
}

fn rotate_log(path: &Path) {
    if fs::metadata(path).map_or(true, |metadata| metadata.len() <= MAX_LOG_SIZE) {
        return;
    }
    let rotated = path.with_extension("jsonl.1");
    let _ = fs::remove_file(&rotated);
    let _ = fs::rename(path, rotated);
}

fn append_entry_to_path(path: &Path, entry: &MessageSendDiagnosticEntry) -> Result<(), String> {
    validate_entry(entry)?;
    let _guard = MESSAGE_SEND_LOG_LOCK
        .lock()
        .map_err(|error| format!("message-send log lock failed: {error}"))?;
    rotate_log(path);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let record = StoredMessageSendDiagnostic {
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        entry,
    };
    serde_json::to_writer(&mut file, &record)
        .map_err(|error| format!("failed to encode message-send diagnostic: {error}"))?;
    writeln!(file).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn append_message_send_diagnostic_internal(app: &AppHandle, entry: MessageSendDiagnosticEntry) {
    if let Ok(path) = message_send_log_path(app) {
        let _ = append_entry_to_path(&path, &entry);
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub(crate) struct MessageSendTrace {
    app: AppHandle,
    operation_id: Option<String>,
    channel_id: String,
    started_at: Instant,
}

impl MessageSendTrace {
    pub fn new(app: AppHandle, operation_id: Option<String>, channel_id: &str) -> Self {
        Self {
            app,
            operation_id,
            channel_id: channel_id.to_string(),
            started_at: Instant::now(),
        }
    }

    fn mark(
        &self,
        stage: String,
        wait_ms: Option<u64>,
        gate_remaining_ms: Option<u64>,
        outcome: Option<&str>,
    ) {
        let Some(operation_id) = &self.operation_id else {
            return;
        };
        append_message_send_diagnostic_internal(
            &self.app,
            MessageSendDiagnosticEntry {
                operation_id: operation_id.clone(),
                stage,
                transport: "http".to_string(),
                channel_id: Some(self.channel_id.clone()),
                event_id: None,
                elapsed_ms: Some(elapsed_ms(self.started_at)),
                wait_ms,
                gate_remaining_ms,
                connection_state: None,
                outcome: outcome.map(str::to_string),
            },
        );
    }

    pub fn started(&self) {
        self.mark("rust_command_started".to_string(), None, None, None);
    }

    pub async fn measure<T>(
        &self,
        stage: &str,
        gate_remaining_ms: Option<u64>,
        future: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        self.mark(format!("{stage}_started"), None, gate_remaining_ms, None);
        let started_at = Instant::now();
        match future.await {
            Ok(value) => {
                self.mark(
                    format!("{stage}_finished"),
                    Some(elapsed_ms(started_at)),
                    gate_remaining_ms,
                    Some("accepted"),
                );
                Ok(value)
            }
            Err(error) => {
                self.mark(
                    format!("{stage}_finished"),
                    Some(elapsed_ms(started_at)),
                    gate_remaining_ms,
                    Some("failed"),
                );
                Err(error)
            }
        }
    }
}

#[tauri::command]
pub async fn append_message_send_diagnostic(
    entry: MessageSendDiagnosticEntry,
    app: AppHandle,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let path = message_send_log_path(&app)?;
        append_entry_to_path(&path, &entry)
    })
    .await
    .map_err(|error| format!("message-send diagnostic task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> MessageSendDiagnosticEntry {
        MessageSendDiagnosticEntry {
            operation_id: "019f-send-probe".into(),
            stage: "relay_ok".into(),
            transport: "websocket".into(),
            channel_id: Some("15fed9f9-a324-5e47-917c-6f33546539b1".into()),
            event_id: Some("ab".repeat(32)),
            elapsed_ms: Some(42),
            wait_ms: None,
            gate_remaining_ms: None,
            connection_state: Some("connected".into()),
            outcome: Some("accepted".into()),
        }
    }

    #[test]
    fn writes_structured_json_without_message_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("message-send.jsonl");
        append_entry_to_path(&path, &entry()).unwrap();
        let record: serde_json::Value =
            serde_json::from_str(fs::read_to_string(path).unwrap().trim()).unwrap();
        assert_eq!(record["stage"], "relay_ok");
        assert_eq!(record["elapsedMs"], 42);
        assert!(record.get("content").is_none());
    }

    #[test]
    fn rejects_free_form_values_that_could_inject_log_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("message-send.jsonl");
        let mut invalid = entry();
        invalid.outcome = Some("failed\nsecret".into());
        assert!(append_entry_to_path(&path, &invalid).is_err());
        assert!(!path.exists());
    }
}
