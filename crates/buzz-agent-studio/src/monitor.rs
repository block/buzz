//! ACP session telemetry → Nostr kind 47300 (Session Monitor).

use serde::{Deserialize, Serialize};

/// Single telemetry snapshot for an agent session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionTelemetry {
    /// Session identifier (ACP session id or run id).
    pub session_id: String,
    /// Agent / persona slug when known.
    pub agent_id: Option<String>,
    /// Cumulative input tokens.
    pub input_tokens: u64,
    /// Cumulative output tokens.
    pub output_tokens: u64,
    /// Estimated USD cost (community-defined pricing).
    pub cost_usd: f64,
    /// Tool invocations in this session.
    pub tool_calls: u32,
    /// Unix timestamp (seconds).
    pub recorded_at: i64,
}

/// In-memory ring buffer for SSE / polling (MVP).
#[derive(Clone, Debug, Default)]
pub struct SessionMonitor {
    max_entries: usize,
    entries: Vec<SessionTelemetry>,
}

impl SessionMonitor {
    /// Create a monitor retaining the last `max_entries` snapshots.
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            entries: Vec::new(),
        }
    }

    /// Record a telemetry snapshot.
    pub fn record(&mut self, telemetry: SessionTelemetry) {
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(telemetry);
    }

    /// Latest snapshots (newest last).
    pub fn snapshots(&self) -> &[SessionTelemetry] {
        &self.entries
    }

    /// Aggregate cost across all retained sessions.
    pub fn total_cost_usd(&self) -> f64 {
        self.entries.iter().map(|e| e.cost_usd).sum()
    }
}

/// JSON array for HTTP API.
pub fn telemetry_json(entries: &[SessionTelemetry]) -> serde_json::Value {
    serde_json::json!({ "sessions": entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_retains_max_entries() {
        let mut monitor = SessionMonitor::new(2);
        for i in 0..3 {
            monitor.record(SessionTelemetry {
                session_id: format!("s{i}"),
                agent_id: None,
                input_tokens: 1,
                output_tokens: 1,
                cost_usd: 0.01,
                tool_calls: 0,
                recorded_at: i,
            });
        }
        assert_eq!(monitor.snapshots().len(), 2);
    }
}
