use serde::{Deserialize, Serialize};

/// Tolerant wire view of an agent profile advertised by the relay.
///
/// `respond_to` deliberately remains an opaque string: third-party harnesses
/// may publish future modes, and one unknown string must not fail the entire
/// relay directory. Desktop interprets only the modes it understands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayAgentInfo {
    pub pubkey: String,
    pub name: String,
    pub agent_type: String,
    pub channels: Vec<String>,
    #[serde(default)]
    pub channel_ids: Vec<String>,
    pub capabilities: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub respond_to: Option<String>,
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn future_respond_to_mode_does_not_fail_its_directory_siblings() {
        let parsed: Vec<RelayAgentInfo> = serde_json::from_value(serde_json::json!([
            {
                "pubkey": "a", "name": "Known", "agent_type": "agent",
                "channels": [], "capabilities": [], "status": "online",
                "respond_to": "anyone"
            },
            {
                "pubkey": "b", "name": "Future", "agent_type": "agent",
                "channels": [], "capabilities": [], "status": "online",
                "respond_to": "future-mode"
            }
        ]))
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].respond_to.as_deref(), Some("future-mode"));
    }
}
