#![deny(unsafe_code)]
//! Audit logging integration for Orbit MCP tools.
//!
//! Maintains a durable record of tool invocations, security checks,
//! and redaction events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// An audit entry representing a single tool invocation or security event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Timestamp of the event.
    pub timestamp: DateTime<Utc>,
    /// Tool name that was invoked.
    pub tool_name: String,
    /// Actor or client identifier.
    pub caller: String,
    /// Execution status ("success", "failed", "blocked").
    pub status: String,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether secret redaction was triggered.
    pub redacted: bool,
    /// Event summary or detail message.
    pub detail: String,
}

/// Durable audit logger writing append-only JSONL entries.
pub struct AuditLogger {
    log_path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLogger {
    /// Creates an audit logger with log file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Self {
        let log_path = path.as_ref().to_path_buf();
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self {
            log_path,
            lock: Mutex::new(()),
        }
    }

    /// Creates an audit logger using default location `~/.orbit/audit.log`.
    pub fn default_local() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home).join(".orbit").join("audit.log");
        Self::new(path)
    }

    /// Records an audit entry.
    pub fn record(&self, entry: &AuditEntry) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(line) = serde_json::to_string(entry) {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                let _ = writeln!(file, "{}", line);
            }
        }
    }

    /// Reads recent audit entries (up to limit).
    pub fn read_recent(&self, limit: usize) -> Vec<AuditEntry> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(content) = std::fs::read_to_string(&self.log_path) {
            let mut entries: Vec<AuditEntry> = content
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            if entries.len() > limit {
                entries.drain(0..entries.len() - limit);
            }
            entries.reverse(); // newest first
            entries
        } else {
            Vec::new()
        }
    }
}
