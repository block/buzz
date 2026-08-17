//! Flow Studio block registry (MVP — ported conceptually from Sim `blocks/registry.ts`).

use serde::{Deserialize, Serialize};

/// Block category for palette grouping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockCategory {
    /// LLM / agent invocation.
    Agent,
    /// Branching logic.
    Condition,
    /// Outbound HTTP call.
    Http,
    /// Inline code execution.
    Code,
    /// Human approval gate (uses buzz-workflow WF-08 when enabled).
    HumanApproval,
}

/// Metadata for a draggable canvas block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockDefinition {
    /// Registry key (snake_case).
    pub block_type: String,
    /// Display name in the palette.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Palette section.
    pub category: BlockCategory,
}

/// MVP block catalog — extended as Sim blocks are ported.
pub fn block_catalog() -> Vec<BlockDefinition> {
    vec![
        BlockDefinition {
            block_type: "agent".into(),
            name: "Agent".into(),
            description: "Run a Buzz persona / managed agent".into(),
            category: BlockCategory::Agent,
        },
        BlockDefinition {
            block_type: "condition".into(),
            name: "Condition".into(),
            description: "Branch on an evalexpr condition".into(),
            category: BlockCategory::Condition,
        },
        BlockDefinition {
            block_type: "http".into(),
            name: "HTTP Request".into(),
            description: "Call an external webhook or REST endpoint".into(),
            category: BlockCategory::Http,
        },
        BlockDefinition {
            block_type: "code".into(),
            name: "Code".into(),
            description: "Execute a sandboxed code step".into(),
            category: BlockCategory::Code,
        },
        BlockDefinition {
            block_type: "human_approval".into(),
            name: "Human Approval".into(),
            description: "Pause until a channel member approves".into(),
            category: BlockCategory::HumanApproval,
        },
    ]
}

/// JSON catalog for `GET /flow-studio/blocks`.
pub fn blocks_json() -> serde_json::Value {
    serde_json::json!({ "blocks": block_catalog() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_mvp_blocks() {
        let blocks = block_catalog();
        assert!(blocks.iter().any(|b| b.block_type == "agent"));
        assert!(blocks.iter().any(|b| b.block_type == "human_approval"));
    }
}
