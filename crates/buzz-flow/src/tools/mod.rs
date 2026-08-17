//! Tool registry for Flow Studio blocks (MVP stub — extended from Sim `tools/`).

use serde::{Deserialize, Serialize};

/// Tool attached to an agent block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowToolDefinition {
    /// Registry key.
    pub tool_type: String,
    /// Display name.
    pub name: String,
    /// Short description.
    pub description: String,
}

/// MVP tool catalog.
pub fn tool_catalog() -> Vec<FlowToolDefinition> {
    vec![
        FlowToolDefinition {
            tool_type: "shell".into(),
            name: "Shell".into(),
            description: "Run a sandboxed shell command".into(),
        },
        FlowToolDefinition {
            tool_type: "web_fetch".into(),
            name: "Web fetch".into(),
            description: "Fetch a URL and return body text".into(),
        },
        FlowToolDefinition {
            tool_type: "buzz_messages".into(),
            name: "Buzz messages".into(),
            description: "Read or post channel messages via relay".into(),
        },
    ]
}

/// JSON catalog for API consumers.
pub fn tools_json() -> serde_json::Value {
    serde_json::json!({ "tools": tool_catalog() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_tools() {
        assert!(!tool_catalog().is_empty());
    }
}
